use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::managed_toolchain::managed_environment_layout::validated_binary_suffix;
use crate::managed_toolchain::{ManagedDestinationBackup, promote_staged_file};
use crate::plan_items::{GoInstallPlanItem, GoInstallSource};

pub(crate) fn execute_go_install_item(
    item: &GoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("go") {
        return Err(OperationError::install("go command not found"));
    }
    let stage_root =
        create_stage_root(managed_dir, "go-install").map_err(OperationError::install)?;
    let expected_destination = managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        validated_binary_suffix(target_triple)
    ));
    let env = vec![("GOBIN".to_string(), stage_root.display().to_string())]
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect::<Vec<_>>();
    let backup = ManagedDestinationBackup::stash(&expected_destination, "go_install binary")
        .map_err(OperationError::install)?;
    let resolved_package = match &item.source {
        GoInstallSource::LocalPath(package_path) => {
            if !package_path.exists() {
                cleanup_stage_root(&stage_root).ok();
                return Err(OperationError::install(format!(
                    "go_install local path does not exist: {}",
                    package_path.display()
                )));
            }
            if !package_path.is_dir() {
                cleanup_stage_root(&stage_root).ok();
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
                cleanup_stage_root(&stage_root).ok();
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
                cleanup_stage_root(&stage_root).ok();
                backup.restore().map_err(OperationError::install)?;
                return Err(OperationError::from_host_recipe(err));
            }
            package.clone()
        }
    };

    let staged_binary = match select_staged_binary(&stage_root, expected_destination.file_name()) {
        Ok(binary) => binary,
        Err(err) => {
            cleanup_stage_root(&stage_root).ok();
            backup.restore().map_err(OperationError::install)?;
            return Err(OperationError::install(err));
        }
    };

    if let Err(err) =
        promote_staged_file(&staged_binary, &expected_destination, "go_install binary")
    {
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
            "expected go_install binary at {}",
            expected_destination.display()
        )));
    }
    backup.discard().map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("go:install:{resolved_package}")),
        source_kind: Some(BootstrapSourceKind::GoInstall),
        archive_match: None,
        destination: Some(expected_destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

fn create_stage_root(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create managed dir {}: {err}", parent.display()))?;
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
            "cannot create staged go_install root {}: {err}",
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
            "cannot clean staged go_install root {}: {err}",
            stage_root.display()
        )
    })
}

fn select_staged_binary(
    stage_root: &Path,
    expected_name: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(stage_root).map_err(|err| {
        format!(
            "go_install succeeded but cannot inspect staged bin dir {}: {err}",
            stage_root.display()
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
            "go_install succeeded but produced no staged binary under {}",
            stage_root.display()
        )),
        _ => Err(format!(
            "go_install produced multiple staged binaries under {} but none matched the requested destination name",
            stage_root.display()
        )),
    }
}
