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
    managed_dir: Option<&Path>,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    match item {
        ResolvedPlanItem::Release(item) => {
            let managed_dir = required_managed_dir(managed_dir, "release")?;
            execute_release_item(item, target_triple, managed_dir, cfg, client).await
        }
        ResolvedPlanItem::ArchiveTreeRelease(item) => {
            let managed_dir = required_managed_dir(managed_dir, "archive_tree_release")?;
            execute_archive_tree_release_item(item, managed_dir, cfg, client).await
        }
        ResolvedPlanItem::SystemPackage(item) => execute_system_package_item(item),
        ResolvedPlanItem::Pip(item) => execute_pip_item(item),
        ResolvedPlanItem::NpmGlobal(item) => {
            let managed_dir = required_managed_dir(managed_dir, "npm_global")?;
            execute_npm_global_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::WorkspacePackage(item) => {
            execute_workspace_package_item(item, managed_dir.unwrap_or_else(|| Path::new("")))
        }
        ResolvedPlanItem::CargoInstall(item) => {
            let managed_dir = required_managed_dir(managed_dir, "cargo_install")?;
            execute_cargo_install_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::RustupComponent(item) => execute_rustup_component_item(
            item,
            target_triple,
            managed_dir.unwrap_or_else(|| Path::new("")),
        ),
        ResolvedPlanItem::GoInstall(item) => {
            let managed_dir = required_managed_dir(managed_dir, "go_install")?;
            execute_go_install_item(item, target_triple, managed_dir)
        }
        ResolvedPlanItem::Uv(_) | ResolvedPlanItem::UvPython(_) | ResolvedPlanItem::UvTool(_) => {
            let managed_dir = required_managed_dir(managed_dir, "managed_toolchain")?;
            execute_managed_toolchain_item(item, target_triple, managed_dir, cfg, client).await
        }
    }
}

fn required_managed_dir<'a>(
    managed_dir: Option<&'a Path>,
    method: &str,
) -> OperationResult<&'a Path> {
    managed_dir.ok_or_else(|| {
        crate::error::OperationError::install(format!(
            "internal error: method `{method}` requires managed_dir"
        ))
    })
}
