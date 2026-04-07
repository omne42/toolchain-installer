use std::path::{Component, Path, PathBuf};

use omne_process_primitives::{resolve_command_path, resolve_command_path_or_standard_location};

use crate::builtin_tools::builtin_tool_selection::is_supported_builtin_tool;
use crate::managed_toolchain::version_probe::binary_reports_version_with_prefix;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManagedBootstrapState {
    NeedsInstall,
    ManagedHealthy { detail: String },
    ManagedBroken { detail: String },
}

pub(crate) fn assess_managed_bootstrap_state(
    tool: &str,
    target_triple: &str,
    destination: &Path,
    managed_dir: &Path,
) -> ManagedBootstrapState {
    if !is_supported_builtin_tool(tool) {
        return ManagedBootstrapState::NeedsInstall;
    }

    if !destination.exists() {
        return ManagedBootstrapState::NeedsInstall;
    }

    if tool == "git" && target_triple.contains("windows") {
        return managed_windows_git_state(managed_dir);
    }

    if managed_binary_reports_expected_version(tool, destination) {
        return ManagedBootstrapState::ManagedHealthy {
            detail: "managed binary passed --version health check".to_string(),
        };
    }

    ManagedBootstrapState::ManagedBroken {
        detail: format!(
            "managed binary exists at {} but failed --version health check",
            destination.display()
        ),
    }
}

pub(crate) fn host_command_is_healthy(tool: &str) -> bool {
    host_command_is_healthy_with_resolver(tool, resolve_command_path)
}

pub(crate) fn host_command_is_healthy_including_standard_locations(tool: &str) -> bool {
    host_command_is_healthy_with_resolver(tool, resolve_command_path_or_standard_location)
}

fn host_command_is_healthy_with_resolver<F>(tool: &str, resolve_command: F) -> bool
where
    F: Fn(&str) -> Option<PathBuf>,
{
    is_supported_builtin_tool(tool)
        && resolve_command(tool)
            .is_some_and(|path| managed_binary_reports_expected_version(tool, &path))
}

fn managed_windows_git_state(managed_dir: &Path) -> ManagedBootstrapState {
    let launcher_path = managed_dir.join("git.cmd");
    let portable_root = managed_dir.join("git-portable");
    let launcher = match std::fs::read_to_string(&launcher_path) {
        Ok(launcher) => launcher,
        Err(err) => {
            return ManagedBootstrapState::ManagedBroken {
                detail: format!(
                    "managed git launcher exists but cannot be read at {}: {err}",
                    launcher_path.display()
                ),
            };
        }
    };
    let Some(relative_target) = launcher_target_from_script(&launcher) else {
        return ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git launcher at {} does not point to a MinGit payload",
                launcher_path.display()
            ),
        };
    };
    let executable =
        match managed_windows_git_payload_path(managed_dir, &portable_root, &relative_target) {
            Ok(executable) => executable,
            Err(detail) => return ManagedBootstrapState::ManagedBroken { detail },
        };
    if !executable.exists() {
        return ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git launcher points to missing MinGit payload {}",
                executable.display()
            ),
        };
    }
    if !managed_binary_reports_expected_version("git", &executable) {
        return ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git payload {} failed --version health check",
                executable.display()
            ),
        };
    }

    ManagedBootstrapState::ManagedHealthy {
        detail: format!(
            "managed git launcher points to healthy MinGit payload {} under {}",
            executable.display(),
            portable_root.display()
        ),
    }
}

fn launcher_target_from_script(script: &str) -> Option<PathBuf> {
    script.lines().find_map(|line| {
        let start = line.find("%~dp0")?;
        let rest = &line[start + 5..];
        let end = rest.find('"')?;
        let target = rest[..end].trim();
        if target.is_empty() {
            return None;
        }
        let mut relative = PathBuf::new();
        for component in target.split(['\\', '/']).filter(|part| !part.is_empty()) {
            relative.push(component);
        }
        (!relative.as_os_str().is_empty()).then_some(relative)
    })
}

fn managed_windows_git_payload_path(
    managed_dir: &Path,
    portable_root: &Path,
    relative_target: &Path,
) -> Result<PathBuf, String> {
    if relative_target.is_absolute()
        || relative_target.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "managed git launcher points outside managed root with payload target `{}`",
            relative_target.display()
        ));
    }
    let executable = managed_dir.join(relative_target);
    if !executable.starts_with(portable_root) {
        return Err(format!(
            "managed git launcher points outside managed git-portable root with payload target `{}`",
            relative_target.display()
        ));
    }
    Ok(executable)
}

fn managed_binary_reports_expected_version(tool: &str, path: &Path) -> bool {
    expected_version_prefix(tool)
        .is_some_and(|expected_prefix| binary_reports_version_with_prefix(path, expected_prefix))
}

fn expected_version_prefix(tool: &str) -> Option<&'static str> {
    match tool {
        "git" => Some("git version "),
        "gh" => Some("gh version "),
        "uv" => Some("uv "),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ManagedBootstrapState, assess_managed_bootstrap_state,
        host_command_is_healthy_with_resolver, launcher_target_from_script,
        managed_windows_git_payload_path,
    };

    #[cfg(unix)]
    fn write_executable(path: &std::path::Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create executable parent");
        }
        std::fs::write(path, body).expect("write executable");
        let mut permissions = std::fs::metadata(path)
            .expect("stat executable")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod executable");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_resolution_can_validate_hidden_supported_binary() {
        let hidden_dir = tempfile::tempdir().expect("hidden tempdir");
        let git_path = hidden_dir.path().join("git");
        write_executable(&git_path, "#!/bin/sh\necho 'git version 2.53.0'\n");

        assert!(host_command_is_healthy_with_resolver("git", |_| Some(
            PathBuf::from(&git_path)
        )));
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_tool_ignores_healthy_managed_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let destination = managed_dir.join("custom-tool");
        write_executable(
            &destination,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "custom-tool 1.0.0"
  exit 0
fi
exit 1
"#,
        );

        assert_eq!(
            assess_managed_bootstrap_state(
                "custom-tool",
                "x86_64-unknown-linux-gnu",
                &destination,
                &managed_dir,
            ),
            ManagedBootstrapState::NeedsInstall
        );
    }

    #[test]
    fn windows_git_health_accepts_mingit_payload_under_git_portable_root() {
        let managed_dir = PathBuf::from("managed");
        let portable_root = managed_dir.join("git-portable");
        let relative_target = PathBuf::from("git-portable")
            .join("mingw64")
            .join("bin")
            .join("git.exe");

        let resolved =
            managed_windows_git_payload_path(&managed_dir, &portable_root, &relative_target)
                .expect("payload under git-portable root should be accepted");

        assert_eq!(
            resolved,
            portable_root.join("mingw64").join("bin").join("git.exe")
        );
    }

    #[test]
    fn windows_git_health_still_accepts_nested_portablegit_payload_layout() {
        let managed_dir = PathBuf::from("managed");
        let portable_root = managed_dir.join("git-portable");
        let relative_target = PathBuf::from("git-portable")
            .join("PortableGit")
            .join("cmd")
            .join("git.exe");

        let resolved =
            managed_windows_git_payload_path(&managed_dir, &portable_root, &relative_target)
                .expect("payload under nested PortableGit root should be accepted");

        assert_eq!(
            resolved,
            portable_root
                .join("PortableGit")
                .join("cmd")
                .join("git.exe")
        );
    }

    #[test]
    fn windows_git_health_rejects_parent_directory_escape() {
        let managed_dir = PathBuf::from("managed");
        let portable_root = managed_dir.join("git-portable");
        let relative_target = PathBuf::from("..").join("outside").join("git.exe");

        let err = managed_windows_git_payload_path(&managed_dir, &portable_root, &relative_target)
            .expect_err("parent directory escape should be rejected");

        assert!(err.contains("outside managed root"));
    }

    #[test]
    fn launcher_target_parser_reads_windows_relative_target() {
        let script = "@echo off\r\n\"%~dp0git-portable\\mingw64\\bin\\git.exe\" %*\r\n";

        assert_eq!(
            launcher_target_from_script(script),
            Some(
                PathBuf::from("git-portable")
                    .join("mingw64")
                    .join("bin")
                    .join("git.exe")
            )
        );
    }
}
