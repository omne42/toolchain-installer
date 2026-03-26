use std::path::Path;

use omne_process_primitives::{HostRecipeRequest, resolve_command_path, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{ResolvedPlanItem, WorkspacePackagePlanItem};

use super::item_destination_resolution::effective_destination_for_item;

pub(crate) fn execute_workspace_package_item(
    item: &WorkspacePackagePlanItem,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    let resolved_item = ResolvedPlanItem::WorkspacePackage(item.clone());
    let workspace_dir = effective_destination_for_item(&resolved_item, "", managed_dir)
        .ok_or_else(|| {
            OperationError::install("workspace_package method requires `destination`")
        })?;
    if !workspace_dir.join("package.json").exists() {
        return Err(OperationError::install(format!(
            "workspace_package requires an existing package.json under {}",
            workspace_dir.display()
        )));
    }
    let args = match item.manager.command_name() {
        "npm" => vec![
            "install".to_string(),
            "--prefix".to_string(),
            workspace_dir.display().to_string(),
            item.package_spec.clone(),
        ],
        "pnpm" => vec![
            "add".to_string(),
            "--dir".to_string(),
            workspace_dir.display().to_string(),
            item.package_spec.clone(),
        ],
        "bun" => vec![
            "add".to_string(),
            "--cwd".to_string(),
            workspace_dir.display().to_string(),
            item.package_spec.clone(),
        ],
        value => {
            return Err(OperationError::install(format!(
                "unsupported workspace_package manager `{value}`"
            )));
        }
    };
    let manager = item.manager.command_name();
    let program = resolve_command_path(manager)
        .and_then(|path| path.into_os_string().into_string().ok())
        .unwrap_or_else(|| manager.to_string());
    run_host_recipe(&HostRecipeRequest::new(program.as_ref(), &args))
        .map_err(OperationError::from_host_recipe)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("workspace:{manager}")),
        source_kind: None,
        archive_match: None,
        destination: Some(workspace_dir.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}
