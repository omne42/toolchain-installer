use std::path::Path;

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::{resolve_command_for_execution, run_recipe};

use super::item_destination_resolution::effective_destination_for_item;

pub(crate) fn execute_workspace_package_item(
    item: &InstallPlanItem,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    let workspace_dir = effective_destination_for_item(item, "", managed_dir).ok_or_else(|| {
        OperationError::install("workspace_package method requires `destination`")
    })?;
    if !workspace_dir.join("package.json").exists() {
        return Err(OperationError::install(format!(
            "workspace_package requires an existing package.json under {}",
            workspace_dir.display()
        )));
    }
    let manager = item
        .manager
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("npm");
    let package = item
        .package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("workspace_package method requires `package`"))?;
    let package = if let Some(version) = item
        .version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if package.contains('@') || package.starts_with("file:") {
            package.to_string()
        } else {
            format!("{package}@{version}")
        }
    } else {
        package.to_string()
    };

    let args = match manager {
        "npm" => vec![
            "install".to_string(),
            "--prefix".to_string(),
            workspace_dir.display().to_string(),
            package,
        ],
        "pnpm" => vec![
            "add".to_string(),
            "--dir".to_string(),
            workspace_dir.display().to_string(),
            package,
        ],
        "bun" => vec![
            "add".to_string(),
            "--cwd".to_string(),
            workspace_dir.display().to_string(),
            package,
        ],
        value => {
            return Err(OperationError::install(format!(
                "unsupported workspace_package manager `{value}`"
            )));
        }
    };
    let program = resolve_command_for_execution(manager);
    run_recipe(&program, &args)?;

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
