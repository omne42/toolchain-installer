use std::path::{Component, Path, PathBuf};

use omne_process_primitives::resolve_command_path_or_standard_location;

use crate::builtin_tools::builtin_tool_selection::is_supported_builtin_tool;
use crate::managed_toolchain::version_probe::binary_reports_version;

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
    if !destination.exists() {
        return ManagedBootstrapState::NeedsInstall;
    }

    if tool == "git" && target_triple.contains("windows") {
        return managed_windows_git_state(managed_dir);
    }

    if managed_binary_reports_version(destination) {
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
    is_supported_builtin_tool(tool)
        && resolve_command_path_or_standard_location(tool)
            .is_some_and(|path| managed_binary_reports_version(&path))
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
    if let Some(expected_dll) = expected_mingit_runtime_dll(&relative_target) {
        let runtime_dll = managed_dir.join(expected_dll);
        if !runtime_dll.exists() {
            return ManagedBootstrapState::ManagedBroken {
                detail: format!(
                    "managed git payload is missing required runtime {}",
                    runtime_dll.display()
                ),
            };
        }
    }
    if !managed_binary_reports_version(&executable) {
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
    let portable_payload_root = portable_root.join("PortableGit");
    if !executable.starts_with(&portable_payload_root) {
        return Err(format!(
            "managed git launcher points outside managed PortableGit payload root with payload target `{}`",
            relative_target.display()
        ));
    }
    Ok(executable)
}

fn expected_mingit_runtime_dll(relative_target: &Path) -> Option<PathBuf> {
    let normalized = relative_target.to_string_lossy().replace('\\', "/");
    if normalized.ends_with("PortableGit/cmd/git.exe") {
        return relative_target
            .parent()
            .and_then(Path::parent)
            .map(|portable_root| {
                portable_root
                    .join("mingw64")
                    .join("bin")
                    .join("msys-2.0.dll")
            });
    }
    if normalized.ends_with("PortableGit/mingw64/bin/git.exe")
        || normalized.ends_with("PortableGit/usr/bin/git.exe")
        || normalized.ends_with("PortableGit/bin/git.exe")
    {
        return relative_target
            .parent()
            .map(|parent| parent.join("msys-2.0.dll"));
    }
    None
}

fn managed_binary_reports_version(path: &Path) -> bool {
    binary_reports_version(path)
}
