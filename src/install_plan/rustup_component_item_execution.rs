use omne_process_primitives::{HostRecipeRequest, resolve_command_path, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::OperationResult;
use crate::plan_items::RustupComponentPlanItem;

pub(crate) fn execute_rustup_component_item(
    item: &RustupComponentPlanItem,
    _target_triple: &str,
    _managed_dir: &std::path::Path,
) -> OperationResult<BootstrapItem> {
    let args = vec![
        "component".to_string(),
        "add".to_string(),
        item.component.to_string(),
    ];
    run_host_recipe(&HostRecipeRequest::new("rustup".as_ref(), &args))
        .map_err(crate::error::OperationError::from_host_recipe)?;

    let destination = item
        .binary_name
        .as_deref()
        .and_then(resolve_command_path)
        .or_else(|| {
            find_rustup_component_binary(&item.component)
                .and_then(|binary| resolve_command_path(&binary))
        });

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("rustup:component:{}", item.component)),
        source_kind: Some(BootstrapSourceKind::RustupComponent),
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
