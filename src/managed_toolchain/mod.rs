mod bootstrap_item_construction;
pub(crate) mod managed_environment_layout;
mod managed_python_executable_discovery;
pub(crate) mod managed_root_dir;
mod managed_uv_installation;
mod source_candidate_attempts;
mod uv_installation_source_candidates;
mod uv_public_release_installation;
mod uv_python_installation;
mod uv_tool_installation;

use std::path::Path;

use crate::contracts::BootstrapItem;
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::plan_items::ResolvedPlanItem;

pub(crate) use uv_public_release_installation::install_uv_from_public_release;

pub(crate) async fn execute_managed_toolchain_item(
    item: &ResolvedPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    match item {
        ResolvedPlanItem::Uv(item) => {
            managed_uv_installation::execute_uv_item(item, target_triple, managed_dir, cfg, client)
                .await
        }
        ResolvedPlanItem::UvPython(item) => {
            uv_python_installation::execute_uv_python_item(
                item,
                target_triple,
                managed_dir,
                cfg,
                client,
            )
            .await
        }
        ResolvedPlanItem::UvTool(item) => {
            uv_tool_installation::execute_uv_tool_item(
                item,
                target_triple,
                managed_dir,
                cfg,
                client,
            )
            .await
        }
        _ => unreachable!("non-managed plan item routed to managed_toolchain execution"),
    }
}

#[cfg(test)]
pub(crate) use managed_python_executable_discovery::find_managed_python_executable;
#[cfg(test)]
pub(crate) use managed_uv_installation::managed_uv_is_healthy;
#[cfg(test)]
pub(crate) use uv_python_installation::execute_uv_python_item;
#[cfg(test)]
pub(crate) use uv_tool_installation::execute_uv_tool_item;
