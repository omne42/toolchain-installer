mod bootstrap_item_construction;
pub(crate) mod managed_environment_layout;
mod managed_python_executable_discovery;
pub(crate) mod managed_root_dir;
mod managed_uv_installation;
mod source_candidate_attempts;
mod uv_installation_source_candidates;
mod uv_python_installation;
mod uv_tool_installation;

use std::path::Path;

use crate::contracts::{BootstrapItem, InstallPlanItem};
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::plan::plan_method::ManagedToolchainMethod;

pub(crate) async fn execute_managed_toolchain_item(
    method: ManagedToolchainMethod,
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    match method {
        ManagedToolchainMethod::Uv => {
            managed_uv_installation::execute_uv_item(item, target_triple, managed_dir, cfg, client)
                .await
        }
        ManagedToolchainMethod::UvPython => {
            uv_python_installation::execute_uv_python_item(
                item,
                target_triple,
                managed_dir,
                cfg,
                client,
            )
            .await
        }
        ManagedToolchainMethod::UvTool => {
            uv_tool_installation::execute_uv_tool_item(
                item,
                target_triple,
                managed_dir,
                cfg,
                client,
            )
            .await
        }
    }
}

#[cfg(test)]
pub(crate) use uv_python_installation::execute_uv_python_item;
#[cfg(test)]
pub(crate) use uv_tool_installation::execute_uv_tool_item;
