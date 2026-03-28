use std::ffi::OsString;
use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;
use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{GoInstallPlanItem, GoInstallSource};

pub(crate) fn execute_go_install_item(
    item: &GoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("go") {
        return Err(OperationError::install("go command not found"));
    }
    let destination = managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ));
    let env = vec![("GOBIN".to_string(), managed_dir.display().to_string())]
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect::<Vec<_>>();
    let backup = InstalledBinaryBackup::stash(&destination).map_err(OperationError::install)?;
    let resolved_package = match &item.source {
        GoInstallSource::LocalPath(package_path) => {
            if !package_path.exists() {
                return Err(OperationError::install(format!(
                    "go_install local path does not exist: {}",
                    package_path.display()
                )));
            }
            if !package_path.is_dir() {
                return Err(OperationError::install(format!(
                    "go_install local path must be a directory: {}",
                    package_path.display()
                )));
            }
            let args = vec!["install".to_string(), ".".to_string()]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            if let Err(err) = run_host_recipe(
                &HostRecipeRequest::new("go".as_ref(), &args)
                    .with_env(&env)
                    .with_working_directory(package_path),
            ) {
                backup.restore().map_err(OperationError::install)?;
                return Err(OperationError::from_host_recipe(err));
            }
            package_path.display().to_string()
        }
        GoInstallSource::PackageSpec(package) => {
            let args = vec!["install".to_string(), package.clone()]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            if let Err(err) =
                run_host_recipe(&HostRecipeRequest::new("go".as_ref(), &args).with_env(&env))
            {
                backup.restore().map_err(OperationError::install)?;
                return Err(OperationError::from_host_recipe(err));
            }
            package.clone()
        }
    };

    if !command_path_exists(&destination) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(format!(
            "expected go_install binary at {}",
            destination.display()
        )));
    }
    backup.discard().map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("go:install:{resolved_package}")),
        source_kind: Some(BootstrapSourceKind::GoInstall),
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
                "cannot stage existing go_install binary backup {}",
                backup.display()
            ));
        }
        std::fs::rename(original, &backup).map_err(|err| {
            format!(
                "cannot stage existing go_install binary {} before reinstall: {err}",
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
                    "cannot remove failed go_install binary {} before restore: {err}",
                    self.original.display()
                )
            })?;
        }
        std::fs::rename(backup, &self.original).map_err(|err| {
            format!(
                "cannot restore previous go_install binary {} from {}: {err}",
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
                "cannot remove staged go_install binary backup {}: {err}",
                backup.display()
            )
        })
    }
}
