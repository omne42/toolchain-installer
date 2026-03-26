use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

use crate::error::{InstallerError, InstallerResult};
use crate::managed_toolchain::managed_environment_layout::managed_python_installation_dir;
use crate::plan_items::{
    CargoInstallPlanItem, GoInstallPlanItem, NodePackageManager, NpmGlobalPlanItem,
    ReleasePlanItem, ResolvedPlanItem, UvToolPlanItem,
};

pub(crate) fn effective_destination_for_item(
    item: &ResolvedPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> Option<PathBuf> {
    match item {
        ResolvedPlanItem::Release(item) => Some(resolve_release_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::ArchiveTreeRelease(item) => Some(
            item.destination
                .as_deref()
                .map(|destination| resolve_destination_path(destination, managed_dir))
                .unwrap_or_else(|| managed_dir.join(&item.id)),
        ),
        ResolvedPlanItem::CargoInstall(item) => Some(resolve_cargo_install_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::NpmGlobal(item) => Some(resolve_npm_global_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::GoInstall(item) => Some(resolve_go_install_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::Uv(_) => {
            Some(managed_dir.join(format!("uv{}", executable_suffix_for_target(target_triple))))
        }
        ResolvedPlanItem::UvPython(_) => Some(managed_python_installation_dir(managed_dir)),
        ResolvedPlanItem::UvTool(item) => Some(resolve_uv_tool_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::WorkspacePackage(item) => {
            Some(resolve_destination_path(&item.destination, managed_dir))
        }
        _ => None,
    }
}

pub(crate) fn resolve_release_destination(
    item: &ReleasePlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    if let Some(destination) = item.destination.as_deref() {
        return resolve_destination_path(destination, managed_dir);
    }
    managed_dir.join(resolve_release_binary_name(item, target_triple))
}

pub(crate) fn resolve_release_binary_name(item: &ReleasePlanItem, target_triple: &str) -> String {
    item.binary_name
        .clone()
        .unwrap_or_else(|| format!("{}{}", item.id, executable_suffix_for_target(target_triple)))
}

pub(crate) fn resolve_cargo_install_destination(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir
        .parent()
        .unwrap_or(managed_dir)
        .join("bin")
        .join(format!(
            "{}{}",
            item.binary_name,
            executable_suffix_for_target(target_triple)
        ))
}

pub(crate) fn resolve_npm_global_destination(
    item: &NpmGlobalPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    match item.manager {
        NodePackageManager::Npm => {
            let prefix_root = if target_triple.contains("windows") {
                managed_dir.to_path_buf()
            } else if managed_dir
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value == "bin")
            {
                managed_dir.parent().unwrap_or(managed_dir).to_path_buf()
            } else {
                managed_dir.to_path_buf()
            };
            if target_triple.contains("windows") {
                prefix_root.join(format!("{}.cmd", item.binary_name))
            } else {
                prefix_root.join("bin").join(&item.binary_name)
            }
        }
        NodePackageManager::Pnpm => managed_dir.join(&item.binary_name),
        NodePackageManager::Bun => managed_dir.join("bin").join(&item.binary_name),
    }
}

pub(crate) fn resolve_go_install_destination(
    item: &GoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ))
}

pub(crate) fn resolve_uv_tool_destination(
    item: &UvToolPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ))
}

pub(crate) fn validate_destination(
    item_id: &str,
    raw_destination: &str,
) -> InstallerResult<PathBuf> {
    let windows_kind = classify_windows_destination(raw_destination);
    match windows_kind {
        WindowsDestinationKind::DriveRelative => {
            return Err(InstallerError::usage(format!(
                "plan item `{item_id}` destination `{raw_destination}` cannot use a Windows drive-relative path such as `C:foo`"
            )));
        }
        WindowsDestinationKind::RootRelative => {
            return Err(InstallerError::usage(format!(
                "plan item `{item_id}` destination `{raw_destination}` cannot use a Windows root-relative path such as `\\foo`"
            )));
        }
        WindowsDestinationKind::Absolute | WindowsDestinationKind::NotWindows => {}
    }

    let path = PathBuf::from(raw_destination);
    if raw_destination.starts_with('/')
        || path.is_absolute()
        || windows_kind == WindowsDestinationKind::Absolute
    {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` destination `{raw_destination}` cannot be an absolute path"
        )));
    }
    if path.file_name().is_none() {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` destination `{raw_destination}` must include a file name"
        )));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` destination `{raw_destination}` cannot contain `..`"
        )));
    }
    Ok(path)
}

pub(crate) fn resolve_destination_path(path: &Path, managed_dir: &Path) -> PathBuf {
    if destination_is_absolute(path) {
        return path.to_path_buf();
    }
    managed_dir.join(path)
}

pub(crate) fn validate_managed_path_boundary(
    destination: &Path,
    managed_dir: &Path,
) -> Result<(), String> {
    if !destination.starts_with(managed_dir) {
        return Ok(());
    }

    reject_symlink_path_component(managed_dir, managed_dir)?;
    let relative = destination
        .strip_prefix(managed_dir)
        .map_err(|err| format!("cannot compute managed-relative destination: {err}"))?;
    let mut current = managed_dir.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        reject_symlink_path_component(&current, managed_dir)?;
    }
    Ok(())
}

fn destination_is_absolute(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }
    matches!(
        classify_windows_destination(path.as_os_str().to_string_lossy().as_ref()),
        WindowsDestinationKind::Absolute
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsDestinationKind {
    Absolute,
    DriveRelative,
    RootRelative,
    NotWindows,
}

fn classify_windows_destination(raw: &str) -> WindowsDestinationKind {
    if raw.starts_with("\\\\") {
        return WindowsDestinationKind::Absolute;
    }
    if raw.starts_with('\\') {
        return WindowsDestinationKind::RootRelative;
    }

    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes
            .get(2)
            .is_some_and(|value| *value == b'\\' || *value == b'/')
        {
            return WindowsDestinationKind::Absolute;
        }
        return WindowsDestinationKind::DriveRelative;
    }

    WindowsDestinationKind::NotWindows
}

fn reject_symlink_path_component(candidate: &Path, managed_dir: &Path) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(candidate) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "cannot inspect managed destination component `{}`: {err}",
                candidate.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "managed destination under `{}` escapes via symlink component `{}`",
            managed_dir.display(),
            candidate.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_destination_rejects_windows_drive_relative_path() {
        let err = validate_destination("demo", "C:tool.exe").expect_err("should reject");
        assert!(
            err.to_string().contains("Windows drive-relative path"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_destination_rejects_windows_root_relative_path() {
        let err = validate_destination("demo", "\\tool.exe").expect_err("should reject");
        assert!(
            err.to_string().contains("Windows root-relative path"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_destination_path_preserves_windows_absolute_paths() {
        let destination =
            resolve_destination_path(Path::new("C:\\tools\\demo.exe"), Path::new("/managed"));
        assert_eq!(destination, PathBuf::from("C:\\tools\\demo.exe"));
    }

    #[test]
    fn validate_destination_rejects_unix_absolute_path() {
        let err = validate_destination("demo", "/tmp/demo").expect_err("should reject");
        assert!(err.to_string().contains("cannot be an absolute path"));
    }

    #[test]
    fn validate_destination_rejects_windows_absolute_path() {
        let err = validate_destination("demo", "C:\\tools\\demo.exe").expect_err("should reject");
        assert!(err.to_string().contains("cannot be an absolute path"));
    }

    #[cfg(unix)]
    #[test]
    fn validate_managed_path_boundary_rejects_symlink_component() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().expect("tempdir");
        let managed_dir = tmp.path().join("managed");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&managed_dir).expect("create managed dir");
        std::fs::create_dir_all(&outside).expect("create outside dir");
        symlink(&outside, managed_dir.join("link")).expect("create symlink");

        let err =
            validate_managed_path_boundary(&managed_dir.join("link").join("demo"), &managed_dir)
                .expect_err("should reject symlink escape");
        assert!(err.contains("escapes via symlink component"));
    }
}
