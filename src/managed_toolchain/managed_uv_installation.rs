use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists,
    resolve_command_path_or_standard_location, run_host_recipe,
};

use crate::artifact::InstallSource;
use crate::contracts::{BootstrapItem, BootstrapSourceKind};
use crate::download_sources::redact_source_url;
use crate::error::{ExitCode, OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::build_installed_bootstrap_item_from_install_source;
use crate::managed_toolchain::install_uv_from_public_release;
use crate::managed_toolchain::managed_environment_layout::{
    bootstrap_uv_binary_path, bootstrap_uv_root, bootstrap_uv_site_packages_dir,
    managed_uv_binary_path,
};
use crate::managed_toolchain::version_probe::binary_reports_version_with_prefix;
use crate::plan_items::ManagedUvPlanItem;

#[derive(Debug, Clone)]
pub(super) struct ManagedUvCommand {
    pub(super) program: PathBuf,
    pub(super) source: InstallSource,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ManagedUvBootstrapMode<'a> {
    ManagedOnly,
    Reusable { preferred_python: Option<&'a str> },
}

pub(crate) async fn execute_uv_item(
    item: &ManagedUvPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let destination = managed_uv_binary_path(target_triple, managed_dir);
    let (uv, detail) = ensure_managed_uv(
        target_triple,
        managed_dir,
        cfg,
        client,
        ManagedUvBootstrapMode::ManagedOnly,
    )
    .await?;
    Ok(build_installed_bootstrap_item_from_install_source(
        &item.id,
        uv.source,
        &destination,
        detail,
    ))
}

pub(super) async fn ensure_managed_uv(
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
    mode: ManagedUvBootstrapMode<'_>,
) -> OperationResult<(ManagedUvCommand, Option<String>)> {
    let destination = managed_uv_binary_path(target_triple, managed_dir);
    let managed_uv_exists = command_path_exists(&destination);
    if managed_uv_exists && managed_uv_is_healthy(&destination) {
        return Ok((
            ManagedUvCommand {
                program: destination,
                source: InstallSource::new("managed", BootstrapSourceKind::Managed),
            },
            Some("managed uv passed --version health check".to_string()),
        ));
    }

    let mut bootstrap_errors = Vec::new();
    if !managed_uv_exists {
        if let ManagedUvBootstrapMode::Reusable { preferred_python } = mode {
            if let Some(host_uv) = healthy_host_uv_command() {
                return Ok((
                    ManagedUvCommand {
                        program: host_uv,
                        source: InstallSource::new("host", BootstrapSourceKind::Managed),
                    },
                    Some("using healthy host uv".to_string()),
                ));
            }

            match bootstrap_uv_from_package_index(target_triple, managed_dir, cfg, preferred_python)
            {
                Ok(Some((uv, detail))) => return Ok((uv, Some(detail))),
                Ok(None) => {}
                Err(err) => bootstrap_errors.push(err.detail()),
            }
        }
    } else if let ManagedUvBootstrapMode::Reusable { .. } = mode {
        bootstrap_errors.push(format!(
            "managed uv at {} failed --version health check",
            destination.display()
        ));
    }

    let source = install_uv_from_public_release(target_triple, &destination, cfg, client)
        .await
        .map_err(|err| append_bootstrap_context(err, &bootstrap_errors))?;
    if !managed_uv_is_healthy(&destination) {
        return Err(OperationError::install(format!(
            "downloaded managed uv at {} but it failed --version health check",
            destination.display()
        )));
    }
    let detail = managed_uv_exists.then(|| {
        format!(
            "reinstalled managed uv after broken binary at {} failed --version health check",
            destination.display()
        )
    });
    Ok((
        ManagedUvCommand {
            program: destination,
            source,
        },
        detail,
    ))
}

pub(crate) fn managed_uv_is_healthy(path: &Path) -> bool {
    binary_reports_version_with_prefix(path, "uv ")
}

fn healthy_host_uv_command() -> Option<PathBuf> {
    let path = resolve_command_path_or_standard_location("uv")?;
    managed_uv_is_healthy(&path).then_some(path)
}

fn bootstrap_uv_from_package_index(
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    preferred_python: Option<&str>,
) -> OperationResult<Option<(ManagedUvCommand, String)>> {
    let python_candidates = managed_uv_python_candidates(preferred_python);
    let bootstrap_root = bootstrap_uv_root(managed_dir);
    let bootstrap_destination = bootstrap_uv_binary_path(target_triple, &bootstrap_root);
    if command_path_exists(&bootstrap_destination) && managed_uv_is_healthy(&bootstrap_destination)
    {
        return Ok(Some((
            ManagedUvCommand {
                program: bootstrap_destination,
                source: InstallSource::new(
                    "package-index bootstrap",
                    BootstrapSourceKind::PackageIndex,
                ),
            },
            "using package-index bootstrapped uv".to_string(),
        )));
    }

    let mut errors = Vec::new();
    for python in python_candidates {
        if !command_exists(&python) {
            errors.push(format!("{python} not found"));
            continue;
        }

        let stage_root =
            bootstrap_stage_root(managed_dir, "uv-bootstrap").map_err(OperationError::install)?;
        if let Err(err) = install_uv_into_stage_with_pip(&python, &stage_root, cfg) {
            remove_dir_if_exists(&stage_root).ok();
            errors.push(format!("{python} failed: {}", err.detail()));
            continue;
        }

        let staged_uv = bootstrap_uv_binary_path(target_triple, &stage_root);
        if !managed_uv_is_healthy(&staged_uv) {
            remove_dir_if_exists(&stage_root).ok();
            errors.push(format!(
                "{python} installed uv bootstrap at {} but it failed --version health check",
                staged_uv.display()
            ));
            continue;
        }

        replace_directory_tree(&stage_root, &bootstrap_root).map_err(OperationError::install)?;
        let detail = format!(
            "bootstrapped reusable uv with `{python} -m pip` from {}",
            primary_package_index_label(cfg)
        );
        return Ok(Some((
            ManagedUvCommand {
                program: bootstrap_uv_binary_path(target_triple, &bootstrap_root),
                source: InstallSource::new(
                    primary_package_index_locator(cfg),
                    BootstrapSourceKind::PackageIndex,
                ),
            },
            detail,
        )));
    }

    if errors.is_empty() {
        return Ok(None);
    }
    Err(OperationError::install(format!(
        "package-index uv bootstrap failed: {}",
        errors.join(" | ")
    )))
}

fn managed_uv_python_candidates(preferred_python: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(preferred_python) = preferred_python
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        candidates.push(preferred_python.to_string());
    }
    candidates.push("python3".to_string());
    candidates.push("python".to_string());
    candidates.dedup();
    candidates
}

fn install_uv_into_stage_with_pip(
    python: &str,
    stage_root: &Path,
    cfg: &InstallerRuntimeConfig,
) -> OperationResult<()> {
    let site_packages_dir = bootstrap_uv_site_packages_dir(stage_root);
    std::fs::create_dir_all(&site_packages_dir).map_err(|err| {
        OperationError::install(format!(
            "create {} failed: {err}",
            site_packages_dir.display()
        ))
    })?;

    let mut args = vec![
        OsString::from("-m"),
        OsString::from("pip"),
        OsString::from("install"),
        OsString::from("--disable-pip-version-check"),
        OsString::from("--upgrade"),
        OsString::from("--target"),
        OsString::from(site_packages_dir.display().to_string()),
    ];
    let mut indexes = cfg
        .package_indexes
        .indexes
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    if let Some(primary) = indexes.next() {
        args.push(OsString::from("--index-url"));
        args.push(OsString::from(primary));
        for extra in indexes {
            args.push(OsString::from("--extra-index-url"));
            args.push(OsString::from(extra));
        }
    }
    args.push(OsString::from("uv"));
    run_host_recipe(&HostRecipeRequest::new(python.as_ref(), &args))
        .map_err(OperationError::from_host_recipe)?;
    write_bootstrap_uv_launcher(stage_root, python)?;
    Ok(())
}

fn write_bootstrap_uv_launcher(bootstrap_root: &Path, python: &str) -> OperationResult<()> {
    let unix_launcher = bootstrap_root.join("bin").join("uv");
    if let Some(parent) = unix_launcher.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            OperationError::install(format!("create {} failed: {err}", parent.display()))
        })?;
    }
    let launcher = format!(
        r#"#!/usr/bin/env bash
set -e
script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
site_packages="$script_dir/../site-packages"
if [ -n "${{PYTHONPATH:-}}" ]; then
  export PYTHONPATH="$site_packages:$PYTHONPATH"
else
  export PYTHONPATH="$site_packages"
fi
exec "{python}" -m uv "$@"
"#
    );
    std::fs::write(&unix_launcher, launcher).map_err(|err| {
        OperationError::install(format!("write {} failed: {err}", unix_launcher.display()))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&unix_launcher)
            .map_err(|err| {
                OperationError::install(format!("stat {} failed: {err}", unix_launcher.display()))
            })?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&unix_launcher, permissions).map_err(|err| {
            OperationError::install(format!("chmod {} failed: {err}", unix_launcher.display()))
        })?;
    }
    #[cfg(windows)]
    {
        let launcher = bootstrap_root.join("Scripts").join("uv.exe");
        if let Some(parent) = launcher.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                OperationError::install(format!("create {} failed: {err}", parent.display()))
            })?;
        }
        let cmd_launcher = launcher.with_extension("cmd");
        let body = format!(
            "@echo off\r\nset SCRIPT_DIR=%~dp0\r\nif defined PYTHONPATH (set PYTHONPATH=%SCRIPT_DIR%..\\site-packages;%PYTHONPATH%) else (set PYTHONPATH=%SCRIPT_DIR%..\\site-packages)\r\n\"{python}\" -m uv %*\r\n"
        );
        std::fs::write(&cmd_launcher, body).map_err(|err| {
            OperationError::install(format!("write {} failed: {err}", cmd_launcher.display()))
        })?;
        std::fs::write(&launcher, b"").map_err(|err| {
            OperationError::install(format!("write {} failed: {err}", launcher.display()))
        })?;
    }
    Ok(())
}

fn bootstrap_stage_root(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
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
    std::fs::create_dir_all(&stage_root)
        .map_err(|err| format!("cannot create {}: {err}", stage_root.display()))?;
    Ok(stage_root)
}

fn replace_directory_tree(staging_root: &Path, destination_root: &Path) -> Result<(), String> {
    let backup_root = destination_root.with_extension("backup");
    remove_dir_if_exists(&backup_root)?;
    if destination_root.exists() {
        std::fs::rename(destination_root, &backup_root)
            .map_err(|err| format!("rename {} failed: {err}", destination_root.display()))?;
    }
    if let Err(err) = std::fs::rename(staging_root, destination_root) {
        if backup_root.exists() {
            let _ = std::fs::rename(&backup_root, destination_root);
        }
        return Err(format!(
            "rename {} to {} failed: {err}",
            staging_root.display(),
            destination_root.display()
        ));
    }
    remove_dir_if_exists(&backup_root)?;
    Ok(())
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("remove {} failed: {err}", path.display()))?;
    }
    Ok(())
}

fn primary_package_index_locator(cfg: &InstallerRuntimeConfig) -> String {
    cfg.package_indexes
        .indexes
        .first()
        .map(|value| redact_source_url(value))
        .unwrap_or_else(|| "package-index".to_string())
}

fn primary_package_index_label(cfg: &InstallerRuntimeConfig) -> String {
    format!("package-index:{}", primary_package_index_locator(cfg))
}

fn append_bootstrap_context(err: OperationError, bootstrap_errors: &[String]) -> OperationError {
    if bootstrap_errors.is_empty() {
        return err;
    }
    let detail = format!(
        "{}; GitHub public release fallback failed: {}",
        bootstrap_errors.join(" | "),
        err.detail()
    );
    match err.exit_code() {
        ExitCode::Download => OperationError::download(detail),
        ExitCode::Usage | ExitCode::Install | ExitCode::StrictFailure => {
            OperationError::install(detail)
        }
    }
}
