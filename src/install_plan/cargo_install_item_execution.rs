use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use omne_process_primitives::{HostRecipeRequest, command_exists, command_path_exists};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::host_recipe::run_installer_host_recipe;
use crate::installer_runtime_config::DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS;
use crate::managed_toolchain::{ManagedDestinationBackup, promote_staged_file};
use crate::plan_items::{CargoInstallPlanItem, CargoInstallSource};

use super::item_destination_resolution::{cargo_install_root, resolve_cargo_install_destination};

#[allow(dead_code)]
pub(crate) fn execute_cargo_install_item(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    execute_cargo_install_item_with_timeout(
        item,
        target_triple,
        managed_dir,
        Duration::from_secs(DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS),
    )
}

pub(crate) fn execute_cargo_install_item_with_timeout(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    timeout: Duration,
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
            push_cargo_install_local_path_arg(&mut args, package_path);
            format!("cargo:path:{}", package_path.display())
        }
        CargoInstallSource::RegistryPackage { package, version } => {
            args.push(OsString::from("--locked"));
            args.push(OsString::from(package));
            if let Some(version) = version.as_deref() {
                args.push(OsString::from("--version"));
                args.push(OsString::from(version));
            }
            format!("cargo:crate:{package}")
        }
    };

    let backup = ManagedDestinationBackup::stash(&expected_destination, "cargo_install binary")
        .map_err(OperationError::install)?;
    if let Err(err) =
        run_installer_host_recipe(&HostRecipeRequest::new("cargo".as_ref(), &args), timeout)
    {
        cleanup_stage_root(&stage_root).ok();
        backup.restore().map_err(OperationError::install)?;
        return Err(err);
    }

    let staged_binary = match select_staged_binary(
        &stage_root.join("bin"),
        expected_destination.file_name(),
        item.binary_name_explicit,
    ) {
        Ok(binary) => binary,
        Err(err) => {
            cleanup_stage_root(&stage_root).ok();
            backup.restore().map_err(OperationError::install)?;
            return Err(OperationError::install(err));
        }
    };

    if let Err(err) = promote_staged_file(
        &staged_binary,
        &expected_destination,
        "cargo_install binary",
    ) {
        cleanup_stage_root(&stage_root).ok();
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
    let detail = build_success_cleanup_detail(
        "cargo_install binary",
        &stage_root,
        &expected_destination,
        || cleanup_stage_root(&stage_root),
    );
    let detail = merge_cleanup_detail(detail, backup.discard_with_warning());

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(source),
        source_kind: Some(BootstrapSourceKind::CargoInstall),
        archive_match: None,
        destination: Some(expected_destination.display().to_string()),
        detail,
        error_code: None,
        failure_code: None,
    })
}

fn build_cargo_install_args(item: &CargoInstallPlanItem, stage_root: &Path) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("install"),
        OsString::from("--root"),
        stage_root.as_os_str().to_os_string(),
    ];
    if item.binary_name_explicit {
        args.push(OsString::from("--bin"));
        args.push(OsString::from(&item.binary_name));
    }
    args
}

fn push_cargo_install_local_path_arg(args: &mut Vec<OsString>, package_path: &Path) {
    args.push(OsString::from("--path"));
    args.push(package_path.as_os_str().to_os_string());
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
    require_expected_name_match: bool,
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

    if let Some(expected_name) = expected_name {
        if let Some(binary) = binaries
            .iter()
            .find(|binary| binary.file_name().is_some_and(|name| name == expected_name))
        {
            return Ok(binary.clone());
        }
        if require_expected_name_match {
            return match binaries.as_slice() {
                [] => Err(format!(
                    "cargo_install succeeded but produced no staged binary under {}",
                    stage_bin_dir.display()
                )),
                [binary] => Err(format!(
                    "cargo_install produced staged binary `{}` under {} but it did not match the requested binary name `{}`",
                    binary
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("<unknown>"),
                    stage_bin_dir.display(),
                    expected_name.to_string_lossy()
                )),
                _ => Err(format!(
                    "cargo_install produced multiple staged binaries under {} but none matched the requested binary name `{}`",
                    stage_bin_dir.display(),
                    expected_name.to_string_lossy()
                )),
            };
        }
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

fn build_success_cleanup_detail(
    label: &str,
    cleanup_target: &Path,
    destination: &Path,
    cleanup: impl FnOnce() -> Result<(), String>,
) -> Option<String> {
    cleanup().err().map(|err| {
        format!(
            "{label} installed at {} but cleanup warning: {err}; staged path `{}` may require manual cleanup",
            destination.display(),
            cleanup_target.display()
        )
    })
}

fn merge_cleanup_detail(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(first), None) => Some(first),
        (None, Some(second)) => Some(second),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use super::{
        build_cargo_install_args, build_success_cleanup_detail, push_cargo_install_local_path_arg,
        select_staged_binary,
    };
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
                OsString::from("install"),
                OsString::from("--root"),
                OsString::from("/tmp/stage"),
                OsString::from("--bin"),
                OsString::from("alias-tool"),
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
                OsString::from("install"),
                OsString::from("--root"),
                OsString::from("/tmp/stage"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn cargo_install_args_preserve_non_utf8_paths() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        use std::path::PathBuf;

        let item = CargoInstallPlanItem {
            id: "demo".to_string(),
            source: CargoInstallSource::RegistryPackage {
                package: "demo".to_string(),
                version: None,
            },
            binary_name: "demo".to_string(),
            binary_name_explicit: false,
        };
        let stage_root = PathBuf::from(OsString::from_vec(b"/tmp/stage-\xff".to_vec()));
        let package_path = PathBuf::from(OsString::from_vec(b"/tmp/pkg-\xfe".to_vec()));

        let mut args = build_cargo_install_args(&item, &stage_root);
        push_cargo_install_local_path_arg(&mut args, &package_path);

        assert_eq!(args[2].as_bytes(), b"/tmp/stage-\xff");
        assert_eq!(args[4].as_bytes(), b"/tmp/pkg-\xfe");
    }

    #[test]
    fn success_cleanup_detail_reports_stage_cleanup_warning() {
        let detail = build_success_cleanup_detail(
            "cargo_install binary",
            Path::new("/tmp/stage"),
            Path::new("/tmp/managed/bin/demo"),
            || Err("cannot clean staged cargo_install root /tmp/stage: busy".to_string()),
        )
        .expect("cleanup warning detail");

        assert!(detail.contains("cleanup warning"));
        assert!(detail.contains("/tmp/stage"));
        assert!(detail.contains("/tmp/managed/bin/demo"));
    }
    #[test]
    fn select_staged_binary_rejects_unique_mismatch_for_explicit_binary_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stage_bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&stage_bin_dir).expect("create staged bin");
        std::fs::write(stage_bin_dir.join("actual-tool"), "binary").expect("write staged binary");

        let err = select_staged_binary(&stage_bin_dir, Some(OsStr::new("alias-tool")), true)
            .expect_err("explicit binary name mismatch should fail");
        assert!(err.contains("did not match the requested binary name `alias-tool`"));
    }

    #[test]
    fn select_staged_binary_keeps_single_fallback_for_inferred_binary_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stage_bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&stage_bin_dir).expect("create staged bin");
        let staged_binary = stage_bin_dir.join("actual-tool");
        std::fs::write(&staged_binary, "binary").expect("write staged binary");

        let selected = select_staged_binary(&stage_bin_dir, Some(OsStr::new("alias-tool")), false)
            .expect("inferred binary name may still fall back to the only staged binary");
        assert_eq!(selected, staged_binary);
    }
}
