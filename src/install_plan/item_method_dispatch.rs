use std::path::Path;

use crate::contracts::BootstrapItem;
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::execute_managed_toolchain_item;
use crate::plan_items::ResolvedPlanItem;

use super::archive_tree_release_item_execution::execute_archive_tree_release_item;
use super::cargo_install_item_execution::execute_cargo_install_item;
use super::go_install_item_execution::execute_go_install_item;
use super::npm_global_item_execution::execute_npm_global_item;
use super::pip_item_execution::execute_pip_item;
use super::release_item_execution::execute_release_item;
use super::rustup_component_item_execution::execute_rustup_component_item;
use super::system_package_item_execution::execute_system_package_item;
use super::workspace_package_item_execution::execute_workspace_package_item;

pub(crate) async fn execute_plan_item(
    item: &ResolvedPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    match item {
        ResolvedPlanItem::Release(item) => {
            execute_release_item(item, target_triple, managed_dir, cfg, client).await
        }
        ResolvedPlanItem::ArchiveTreeRelease(item) => {
            execute_archive_tree_release_item(item, managed_dir, cfg, client).await
        }
        ResolvedPlanItem::SystemPackage(item) => execute_system_package_item(item),
        ResolvedPlanItem::Pip(item) => execute_pip_item(item),
        ResolvedPlanItem::NpmGlobal(item) => {
            execute_npm_global_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::WorkspacePackage(item) => {
            execute_workspace_package_item(item, managed_dir)
        }
        ResolvedPlanItem::CargoInstall(item) => {
            execute_cargo_install_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::RustupComponent(item) => {
            execute_rustup_component_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::GoInstall(item) => {
            execute_go_install_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::Uv(_) | ResolvedPlanItem::UvPython(_) | ResolvedPlanItem::UvTool(_) => {
            execute_managed_toolchain_item(item, target_triple, managed_dir, cfg, client).await
        }
    }
}
