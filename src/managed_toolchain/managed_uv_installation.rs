use std::path::{Path, PathBuf};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, InstallPlanItem, InstallSource};
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::build_installed_bootstrap_item_from_install_source;
use crate::managed_toolchain::managed_environment_layout::managed_uv_binary_path;
use crate::platform::process_runner::command_path_exists;
use crate::uv::release_installation::install_uv_from_public;

#[derive(Debug, Clone)]
pub(super) struct ManagedUvCommand {
    pub(super) program: PathBuf,
    pub(super) source: InstallSource,
}

pub(crate) async fn execute_uv_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let destination = managed_uv_binary_path(target_triple, managed_dir);
    let (uv, detail) = ensure_managed_uv(target_triple, managed_dir, cfg, client).await?;
    Ok(build_installed_bootstrap_item_from_install_source(
        item,
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
    if command_path_exists(&destination) {
        return Ok((
            ManagedUvCommand {
                program: destination,
                source: InstallSource::new("managed", BootstrapSourceKind::Managed),
            },
            Some("managed uv already exists".to_string()),
        ));
    }

    let source = install_uv_from_public(target_triple, &destination, cfg, client).await?;
    Ok((
        ManagedUvCommand {
            program: destination,
            source,
        },
        None,
    ))
}
