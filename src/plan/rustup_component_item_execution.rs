use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::{resolve_command_path, run_recipe};

pub(crate) fn execute_rustup_component_item(
    item: &InstallPlanItem,
    _target_triple: &str,
    _managed_dir: &std::path::Path,
) -> OperationResult<BootstrapItem> {
    let component = item
        .package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("rustup_component method requires `package`"))?;
    let args = vec![
        "component".to_string(),
        "add".to_string(),
        component.to_string(),
    ];
    run_recipe("rustup", &args)?;

    let destination = item
        .binary_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(resolve_command_path)
        .or_else(|| {
            find_rustup_component_binary(component).and_then(|binary| resolve_command_path(&binary))
        });

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("rustup:component:{component}")),
        source_kind: None,
        archive_match: None,
        destination: destination.map(|path| path.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

fn find_rustup_component_binary(component: &str) -> Option<String> {
    let binary_name = match component {
        "rustfmt" => "rustfmt",
        "clippy" => "cargo-clippy",
        _ => return None,
    };
    Some(binary_name.to_string())
}
