use std::path::Path;

use crate::contracts::BootstrapItem;
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::execute_managed_toolchain_item;
use crate::plan_items::ResolvedPlanItem;

use super::archive_tree_release_item_execution::execute_archive_tree_release_item;
use super::cargo_install_item_execution::execute_cargo_install_item_with_timeout;
use super::go_install_item_execution::execute_go_install_item_with_timeout;
use super::npm_global_item_execution::execute_npm_global_item_with_timeout;
use super::pip_item_execution::execute_pip_item;
use super::release_item_execution::execute_release_item;
use super::rustup_component_item_execution::execute_rustup_component_item_with_timeout;
use super::system_package_item_execution::execute_system_package_item_with_timeout;
use super::workspace_package_item_execution::execute_workspace_package_item_with_timeout;

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
        ResolvedPlanItem::SystemPackage(item) => {
            execute_system_package_item_with_timeout(item, cfg.host_recipes.timeout)
        }
        ResolvedPlanItem::Pip(item) => execute_pip_item(item, cfg.host_recipes.timeout),
        ResolvedPlanItem::NpmGlobal(item) => {
            let managed_dir = required_managed_dir(managed_dir, "npm_global")?;
            execute_npm_global_item_with_timeout(
                item,
                target_triple,
                managed_dir,
                cfg.host_recipes.timeout,
            )
        }
        ResolvedPlanItem::WorkspacePackage(item) => execute_workspace_package_item_with_timeout(
            item,
            managed_dir.unwrap_or_else(|| Path::new("")),
            cfg.host_recipes.timeout,
        ),
        ResolvedPlanItem::CargoInstall(item) => {
            let managed_dir = required_managed_dir(managed_dir, "cargo_install")?;
            execute_cargo_install_item_with_timeout(
                item,
                target_triple,
                managed_dir,
                cfg.host_recipes.timeout,
            )
        }
        ResolvedPlanItem::RustupComponent(item) => execute_rustup_component_item_with_timeout(
            item,
            target_triple,
            managed_dir.unwrap_or_else(|| Path::new("")),
            cfg.host_recipes.timeout,
        ),
        ResolvedPlanItem::GoInstall(item) => {
            let managed_dir = required_managed_dir(managed_dir, "go_install")?;
            execute_go_install_item_with_timeout(
                item,
                target_triple,
                managed_dir,
                cfg.host_recipes.timeout,
            )
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
