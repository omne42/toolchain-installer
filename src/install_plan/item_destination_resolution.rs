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
        ResolvedPlanItem::WorkspacePackage(item) => Some(normalize_lexical_path(&item.destination)),
        _ => None,
    }
}

pub(crate) fn allow_leaf_symlink_in_managed_destination(item: &ResolvedPlanItem) -> bool {
    matches!(item, ResolvedPlanItem::NpmGlobal(_))
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
    cargo_install_root(managed_dir).join("bin").join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ))
}

pub(crate) fn cargo_install_root(managed_dir: &Path) -> PathBuf {
    if managed_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == "bin")
    {
        return managed_dir.parent().unwrap_or(managed_dir).to_path_buf();
    }
    managed_dir.to_path_buf()
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
                prefix_root.join(npm_global_binary_filename(&item.binary_name, target_triple))
            } else {
                prefix_root.join("bin").join(&item.binary_name)
            }
        }
        NodePackageManager::Pnpm => {
            managed_dir.join(npm_global_binary_filename(&item.binary_name, target_triple))
        }
        NodePackageManager::Bun => managed_dir
            .join("bin")
            .join(npm_global_binary_filename(&item.binary_name, target_triple)),
    }
}

fn npm_global_binary_filename(binary_name: &str, target_triple: &str) -> String {
    if target_triple.contains("windows") {
        return format!("{binary_name}.cmd");
    }
    binary_name.to_string()
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
    target_triple: &str,
) -> InstallerResult<PathBuf> {
    let path = validate_destination_path(
        item_id,
        raw_destination,
        target_triple,
        DestinationPolicy::Managed,
    )?;
    Ok(normalize_lexical_path(&path))
}

pub(crate) fn validate_workspace_destination(
    item_id: &str,
    raw_destination: &str,
    target_triple: &str,
) -> InstallerResult<PathBuf> {
    let path = validate_destination_path(
        item_id,
        raw_destination,
        target_triple,
        DestinationPolicy::Workspace,
    )?;
    Ok(normalize_lexical_path(&path))
}

fn validate_destination_path(
    item_id: &str,
    raw_destination: &str,
    target_triple: &str,
    policy: DestinationPolicy,
) -> InstallerResult<PathBuf> {
    if raw_destination.starts_with('/') {
        if matches!(policy, DestinationPolicy::Managed) {
            return Err(InstallerError::usage(format!(
                "plan item `{item_id}` destination `{raw_destination}` cannot be an absolute path"
            )));
        }
        let path = PathBuf::from(raw_destination);
        validate_parsed_destination(item_id, raw_destination, &path)?;
        return Ok(path);
    }

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
        WindowsDestinationKind::Absolute => {
            if windows_destination_has_no_file_name(raw_destination) {
                return Err(InstallerError::usage(format!(
                    "plan item `{item_id}` destination `{raw_destination}` must include a file name"
                )));
            }
            if windows_destination_has_parent_component(raw_destination) {
                return Err(InstallerError::usage(format!(
                    "plan item `{item_id}` destination `{raw_destination}` cannot contain `..`"
                )));
            }
            let path = PathBuf::from(raw_destination);
            validate_parsed_destination(item_id, raw_destination, &path)?;
            return Ok(path);
        }
        WindowsDestinationKind::NotWindows => {}
    }

    let path = if target_triple.contains("windows") {
        parse_windows_relative_path(raw_destination)
    } else {
        PathBuf::from(raw_destination)
    };
    if matches!(policy, DestinationPolicy::Managed) && (path.is_absolute() || path.has_root()) {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` destination `{raw_destination}` cannot be an absolute path"
        )));
    }
    validate_parsed_destination(item_id, raw_destination, &path)?;
    Ok(path)
}

fn parse_windows_relative_path(raw_destination: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for component in raw_destination.split(['\\', '/']) {
        if component.is_empty() || component == "." {
            continue;
        }
        path.push(component);
    }
    path
}

fn validate_parsed_destination(
    item_id: &str,
    raw_destination: &str,
    path: &Path,
) -> InstallerResult<()> {
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
    Ok(())
}

pub(crate) fn resolve_destination_path(path: &Path, managed_dir: &Path) -> PathBuf {
    if destination_is_absolute(path) {
        return normalize_lexical_path(path);
    }
    normalize_lexical_path(&managed_dir.join(path))
}

pub(crate) fn resolve_plan_relative_path(path: &Path, plan_base_dir: Option<&Path>) -> PathBuf {
    if destination_is_absolute(path) {
        return normalize_lexical_path(path);
    }
    if let Some(base_dir) = plan_base_dir {
        return normalize_lexical_path(&base_dir.join(path));
    }
    normalize_lexical_path(path)
}

pub(crate) fn validate_managed_path_boundary(
    destination: &Path,
    managed_dir: &Path,
    allow_leaf_symlink: bool,
) -> Result<(), String> {
    if !destination.starts_with(managed_dir) {
        return Ok(());
    }

    reject_symlink_path_component(managed_dir, managed_dir)?;
    let relative = destination
        .strip_prefix(managed_dir)
        .map_err(|err| format!("cannot compute managed-relative destination: {err}"))?;
    let components: Vec<_> = relative.components().collect();
    let mut current = managed_dir.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        if allow_leaf_symlink && index + 1 == components.len() {
            continue;
        }
        reject_symlink_path_component(&current, managed_dir)?;
    }
    Ok(())
}

pub(crate) fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::CurDir) {
            continue;
        }
        normalized.push(component.as_os_str());
    }
    normalized
}

fn destination_is_absolute(path: &Path) -> bool {
    if path.is_absolute() || path.has_root() {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DestinationPolicy {
    Managed,
    Workspace,
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

fn windows_destination_has_no_file_name(raw: &str) -> bool {
    raw.split(['\\', '/'])
        .rfind(|component| !component.is_empty())
        .is_none_or(|component| component == "." || component == "..")
}

fn windows_destination_has_parent_component(raw: &str) -> bool {
    raw.split(['\\', '/']).any(|component| component == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_destination_rejects_windows_drive_relative_path() {
        let err = validate_destination("demo", "C:tool.exe", "x86_64-pc-windows-msvc")
            .expect_err("should reject");
        assert!(
            err.to_string().contains("Windows drive-relative path"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_destination_rejects_windows_root_relative_path() {
        let err = validate_destination("demo", "\\tool.exe", "x86_64-pc-windows-msvc")
            .expect_err("should reject");
        assert!(
            err.to_string().contains("Windows root-relative path"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn cargo_install_root_stays_within_custom_managed_dir() {
        let managed_dir = Path::new("/tmp/custom-managed");
        assert_eq!(
            cargo_install_root(managed_dir),
            managed_dir,
            "custom managed_dir should remain the cargo install root"
        );
        assert_eq!(
            resolve_cargo_install_destination(
                &CargoInstallPlanItem {
                    id: "demo".to_string(),
                    source: crate::plan_items::CargoInstallSource::RegistryPackage {
                        package: "demo".to_string(),
                        version: None,
                    },
                    binary_name: "demo".to_string(),
                    binary_name_explicit: false,
                },
                "x86_64-unknown-linux-gnu",
                managed_dir,
            ),
            managed_dir.join("bin").join("demo")
        );
    }

    #[test]
    fn resolve_destination_path_preserves_windows_absolute_paths() {
        let destination =
            resolve_destination_path(Path::new("C:\\tools\\demo.exe"), Path::new("/managed"));
        assert_eq!(destination, PathBuf::from("C:\\tools\\demo.exe"));
    }

    #[test]
    fn resolve_npm_global_destination_uses_windows_cmd_entrypoints_for_pnpm_and_bun() {
        let managed_dir = Path::new(r"C:\managed");
        let pnpm_destination = resolve_npm_global_destination(
            &NpmGlobalPlanItem {
                id: "pnpm-demo".to_string(),
                package_spec: "demo".to_string(),
                manager: NodePackageManager::Pnpm,
                binary_name: "demo".to_string(),
            },
            "x86_64-pc-windows-msvc",
            managed_dir,
        );
        assert_eq!(
            pnpm_destination,
            PathBuf::from(r"C:\managed").join("demo.cmd")
        );

        let bun_destination = resolve_npm_global_destination(
            &NpmGlobalPlanItem {
                id: "bun-demo".to_string(),
                package_spec: "demo".to_string(),
                manager: NodePackageManager::Bun,
                binary_name: "demo".to_string(),
            },
            "x86_64-pc-windows-msvc",
            managed_dir,
        );
        assert_eq!(
            bun_destination,
            PathBuf::from(r"C:\managed").join("bin").join("demo.cmd")
        );
    }

    #[test]
    fn validate_destination_rejects_unix_absolute_path() {
        let err = validate_destination("demo", "/tmp/demo", "x86_64-unknown-linux-gnu")
            .expect_err("should reject");
        assert!(err.to_string().contains("cannot be an absolute path"));
    }

    #[test]
    fn validate_destination_rejects_forward_slash_root_path_for_windows_targets() {
        let err = validate_destination("demo", "/tools/demo.exe", "x86_64-pc-windows-msvc")
            .expect_err("should reject");
        assert!(err.to_string().contains("cannot be an absolute path"));
    }

    #[test]
    fn validate_destination_accepts_windows_absolute_path_for_managed_installs() {
        let destination =
            validate_destination("demo", "C:\\tools\\demo.exe", "x86_64-pc-windows-msvc")
                .expect("windows absolute destination");
        assert_eq!(destination, PathBuf::from("C:\\tools\\demo.exe"));
    }

    #[test]
    fn validate_destination_rejects_parent_components_in_windows_absolute_path() {
        let err = validate_workspace_destination(
            "demo",
            "C:\\tools\\..\\demo.exe",
            "x86_64-pc-windows-msvc",
        )
        .expect_err("should reject");
        assert!(err.to_string().contains("cannot contain `..`"));
    }

    #[test]
    fn validate_workspace_destination_accepts_absolute_path() {
        let destination =
            validate_workspace_destination("demo", "/workspace/app", "x86_64-unknown-linux-gnu")
                .expect("absolute workspace");
        assert_eq!(destination, PathBuf::from("/workspace/app"));
    }

    #[test]
    fn validate_workspace_destination_accepts_windows_absolute_path() {
        let destination =
            validate_workspace_destination("demo", "C:\\workspace\\app", "x86_64-pc-windows-msvc")
                .expect("windows absolute workspace");
        assert_eq!(destination, PathBuf::from("C:\\workspace\\app"));
    }

    #[test]
    fn validate_destination_normalizes_windows_relative_path_for_windows_targets() {
        let destination =
            validate_destination("demo", "bin\\tools\\demo.exe", "x86_64-pc-windows-msvc")
                .expect("windows relative destination");
        assert_eq!(
            destination,
            PathBuf::from("bin").join("tools").join("demo.exe")
        );
    }

    #[test]
    fn resolve_plan_relative_path_uses_plan_base_directory() {
        let path =
            resolve_plan_relative_path(Path::new("./packages/app"), Some(Path::new("/repo")));
        assert_eq!(path, PathBuf::from("/repo/packages/app"));
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

        let err = validate_managed_path_boundary(
            &managed_dir.join("link").join("demo"),
            &managed_dir,
            false,
        )
        .expect_err("should reject symlink escape");
        assert!(err.contains("escapes via symlink component"));
    }

    #[cfg(unix)]
    #[test]
    fn validate_managed_path_boundary_allows_leaf_symlink_when_requested() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().expect("tempdir");
        let managed_dir = tmp.path().join("managed");
        let package_dir = managed_dir
            .join("lib")
            .join("node_modules")
            .join("demo")
            .join("bin");
        std::fs::create_dir_all(&package_dir).expect("create package dir");
        std::fs::create_dir_all(managed_dir.join("bin")).expect("create bin dir");
        symlink(
            package_dir.join("demo"),
            managed_dir.join("bin").join("demo"),
        )
        .expect("create symlink");

        validate_managed_path_boundary(&managed_dir.join("bin").join("demo"), &managed_dir, true)
            .expect("leaf symlink should be allowed");
    }
}
