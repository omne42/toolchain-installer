use std::path::{Path, PathBuf};

use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
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
    let destination = resolve_cargo_install_destination(item, target_triple, managed_dir);

    let mut args = vec!["install".to_string()];
    let source = match &item.source {
        CargoInstallSource::LocalPath(package_path) => {
            if !package_path.exists() {
                return Err(OperationError::install(format!(
                    "cargo_install local path does not exist: {}",
                    package_path.display()
                )));
            }
            if !package_path.is_dir() {
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
    args.push("--root".to_string());
    args.push(install_root.display().to_string());
    let backup = InstalledBinaryBackup::stash(&destination).map_err(OperationError::install)?;
    if let Err(err) = run_host_recipe(&HostRecipeRequest::new("cargo".as_ref(), &args)) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::from_host_recipe(err));
    }

    if !command_path_exists(&destination) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(format!(
            "expected cargo_install binary at {}",
            destination.display()
        )));
    }
    backup.discard().map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(source),
        source_kind: None,
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
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
