use std::path::Path;

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::execute_managed_toolchain_item;

use super::pip_item_execution::execute_pip_item;
use super::plan_method::PlanMethod;
use super::release_item_execution::execute_release_item;
use super::system_package_item_execution::execute_system_package_item;

pub(crate) async fn execute_plan_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    match PlanMethod::classify(item).unwrap_or(PlanMethod::Unknown) {
        PlanMethod::Release => {
            execute_release_item(item, target_triple, managed_dir, cfg, client).await
        }
        PlanMethod::Apt | PlanMethod::SystemPackage => execute_system_package_item(item),
        PlanMethod::Pip => execute_pip_item(item),
        PlanMethod::ManagedToolchain(method) => {
            execute_managed_toolchain_item(method, item, target_triple, managed_dir, cfg, client)
                .await
        }
        PlanMethod::Unknown => Ok(BootstrapItem {
            tool: item.id.clone(),
            status: BootstrapStatus::Unsupported,
            source: None,
            source_kind: None,
            archive_match: None,
            destination: item.destination.clone(),
            detail: Some(format!("unsupported plan method `{}`", item.method)),
            error_code: None,
            failure_code: None,
        }),
    }
}
