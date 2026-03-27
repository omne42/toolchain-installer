use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use omne_process_primitives::command_path_exists;

use crate::artifact::InstallSource;
use crate::contracts::{BootstrapItem, BootstrapSourceKind};
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::build_installed_bootstrap_item_from_install_source;
use crate::managed_toolchain::install_uv_from_public_release;
use crate::managed_toolchain::managed_environment_layout::managed_uv_binary_path;
use crate::plan_items::ManagedUvPlanItem;

#[derive(Debug, Clone)]
pub(super) struct ManagedUvCommand {
    pub(super) program: PathBuf,
    pub(super) source: InstallSource,
}

const MANAGED_UV_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn execute_uv_item(
    item: &ManagedUvPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let destination = managed_uv_binary_path(target_triple, managed_dir);
    let (uv, detail) = ensure_managed_uv(target_triple, managed_dir, cfg, client).await?;
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

    let source = install_uv_from_public_release(target_triple, &destination, cfg, client).await?;
    let detail = if managed_uv_exists {
        Some(format!(
            "reinstalled managed uv at {} after failed --version health check",
            destination.display()
        ))
    } else {
        None
    };
    Ok((
        ManagedUvCommand {
            program: destination,
            source,
        },
        detail,
    ))
}

pub(crate) fn managed_uv_is_healthy(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    let Ok(mut child) = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    let deadline = Instant::now() + MANAGED_UV_HEALTH_CHECK_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
