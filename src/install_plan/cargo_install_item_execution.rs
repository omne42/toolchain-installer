use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{CargoInstallPlanItem, CargoInstallSource};

use super::item_destination_resolution::{cargo_install_root, resolve_cargo_install_destination};

pub(crate) fn execute_cargo_install_item(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("cargo") {
        return Err(OperationError::install("cargo command not found"));
    }
    let install_root = cargo_install_root(managed_dir);
    let stage_root =
        create_stage_root(&install_root, "cargo-install").map_err(OperationError::install)?;
    let expected_destination = resolve_cargo_install_destination(item, target_triple, managed_dir);

    let mut args = build_cargo_install_args(item, &stage_root);
    let source = match &item.source {
        CargoInstallSource::LocalPath(package_path) => {
            if !package_path.exists() {
                cleanup_stage_root(&stage_root).ok();
                return Err(OperationError::install(format!(
                    "cargo_install local path does not exist: {}",
                    package_path.display()
                )));
            }
            if !package_path.is_dir() {
                cleanup_stage_root(&stage_root).ok();
                return Err(OperationError::install(format!(
                    "cargo_install local path must be a directory: {}",
                    package_path.display()
                )));
            }
            args.push("--path".to_string());
            args.push(package_path.display().to_string());
            format!("cargo:path:{}", package_path.display())
        }
        CargoInstallSource::RegistryPackage { package, version } => {
            args.push("--locked".to_string());
            args.push(package.clone());
            if let Some(version) = version.as_deref() {
                args.push("--version".to_string());
                args.push(version.to_string());
            }
            format!("cargo:crate:{package}")
        }
    };
    let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();

    let backup =
        InstalledBinaryBackup::stash(&expected_destination).map_err(OperationError::install)?;
    if let Err(err) = run_host_recipe(&HostRecipeRequest::new("cargo".as_ref(), &args)) {
        cleanup_stage_root(&stage_root).ok();
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::from_host_recipe(err));
    }

    let staged_binary =
        match select_staged_binary(&stage_root.join("bin"), expected_destination.file_name()) {
            Ok(binary) => binary,
            Err(err) => {
                cleanup_stage_root(&stage_root).ok();
                backup.restore().map_err(OperationError::install)?;
                return Err(OperationError::install(err));
            }
        };

    if let Err(err) = promote_staged_binary(&staged_binary, &expected_destination) {
        cleanup_stage_root(&stage_root).ok();
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(err));
    }
    if let Err(err) = cleanup_stage_root(&stage_root) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(err));
    }
    if !command_path_exists(&expected_destination) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(format!(
            "expected cargo_install binary at {}",
            expected_destination.display()
        )));
    }
    backup.discard().map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(source),
        source_kind: Some(BootstrapSourceKind::CargoInstall),
        archive_match: None,
        destination: Some(expected_destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

fn build_cargo_install_args(item: &CargoInstallPlanItem, stage_root: &Path) -> Vec<String> {
    let mut args = vec![
        "install".to_string(),
        "--root".to_string(),
        stage_root.display().to_string(),
    ];
    if item.binary_name_explicit {
        args.push("--bin".to_string());
        args.push(item.binary_name.clone());
    }
    args
}

fn create_stage_root(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create install root {}: {err}", parent.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stage_root = parent.join(format!(
        ".toolchain-installer-{prefix}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&stage_root).map_err(|err| {
        format!(
            "cannot create staged cargo_install root {}: {err}",
            stage_root.display()
        )
    })?;
    Ok(stage_root)
}

fn cleanup_stage_root(stage_root: &Path) -> Result<(), String> {
    if !stage_root.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(stage_root).map_err(|err| {
        format!(
            "cannot clean staged cargo_install root {}: {err}",
            stage_root.display()
        )
    })
}

fn select_staged_binary(
    stage_bin_dir: &Path,
    expected_name: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(stage_bin_dir).map_err(|err| {
        format!(
            "cargo_install succeeded but cannot inspect staged bin dir {}: {err}",
            stage_bin_dir.display()
        )
    })?;
    let mut binaries = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            entry.file_type().ok()?.is_file().then_some(path)
        })
        .collect::<Vec<_>>();
    binaries.sort();

    if let Some(expected_name) = expected_name
        && let Some(binary) = binaries
            .iter()
            .find(|binary| binary.file_name().is_some_and(|name| name == expected_name))
    {
        return Ok(binary.clone());
    }

    match binaries.as_slice() {
        [binary] => Ok(binary.clone()),
        [] => Err(format!(
            "cargo_install succeeded but produced no staged binary under {}",
            stage_bin_dir.display()
        )),
        _ => Err(format!(
            "cargo_install produced multiple staged binaries under {} but none matched the requested destination name",
            stage_bin_dir.display()
        )),
    }
}

fn promote_staged_binary(staged_binary: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "cannot create cargo_install destination parent {}: {err}",
                parent.display()
            )
        })?;
    }
    if destination.exists() {
        std::fs::remove_file(destination).map_err(|err| {
            format!(
                "cannot remove existing cargo_install binary {}: {err}",
                destination.display()
            )
        })?;
    }
    std::fs::rename(staged_binary, destination).map_err(|err| {
        format!(
            "cannot promote staged cargo_install binary {} to {}: {err}",
            staged_binary.display(),
            destination.display()
        )
    })
}

struct InstalledBinaryBackup {
    original: PathBuf,
    backup: Option<PathBuf>,
}

impl InstalledBinaryBackup {
    fn stash(original: &Path) -> Result<Self, String> {
        if !original.exists() {
            return Ok(Self {
                original: original.to_path_buf(),
                backup: None,
            });
        }

        let backup = original.with_file_name(format!(
            "{}.toolchain-installer-backup",
            original
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("managed-tool")
        ));
        if backup.exists() {
            return Err(format!(
                "cannot stage existing cargo_install binary backup {}",
                backup.display()
            ));
        }
        std::fs::rename(original, &backup).map_err(|err| {
            format!(
                "cannot stage existing cargo_install binary {} before reinstall: {err}",
                original.display()
            )
        })?;
        Ok(Self {
            original: original.to_path_buf(),
            backup: Some(backup),
        })
    }

    fn restore(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        if self.original.exists() {
            std::fs::remove_file(&self.original).map_err(|err| {
                format!(
                    "cannot remove failed cargo_install binary {} before restore: {err}",
                    self.original.display()
                )
            })?;
        }
        std::fs::rename(backup, &self.original).map_err(|err| {
            format!(
                "cannot restore previous cargo_install binary {} from {}: {err}",
                self.original.display(),
                backup.display()
            )
        })
    }

    fn discard(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        std::fs::remove_file(backup).map_err(|err| {
            format!(
                "cannot remove staged cargo_install binary backup {}: {err}",
                backup.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::build_cargo_install_args;
    use crate::plan_items::{CargoInstallPlanItem, CargoInstallSource};

    #[test]
    fn explicit_binary_name_enters_cargo_install_args() {
        let item = CargoInstallPlanItem {
            id: "demo".to_string(),
            source: CargoInstallSource::RegistryPackage {
                package: "demo".to_string(),
                version: None,
            },
            binary_name: "alias-tool".to_string(),
            binary_name_explicit: true,
        };

        let args = build_cargo_install_args(&item, Path::new("/tmp/stage"));
        assert_eq!(
            args,
            vec![
                "install".to_string(),
                "--root".to_string(),
                "/tmp/stage".to_string(),
                "--bin".to_string(),
                "alias-tool".to_string(),
            ]
        );
    }

    #[test]
    fn inferred_binary_name_does_not_force_cargo_bin_arg() {
        let item = CargoInstallPlanItem {
            id: "demo".to_string(),
            source: CargoInstallSource::RegistryPackage {
                package: "demo".to_string(),
                version: None,
            },
            binary_name: "demo".to_string(),
            binary_name_explicit: false,
        };

        let args = build_cargo_install_args(&item, Path::new("/tmp/stage"));
        assert_eq!(
            args,
            vec![
                "install".to_string(),
                "--root".to_string(),
                "/tmp/stage".to_string(),
            ]
        );
    }
}
