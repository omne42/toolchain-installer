use omne_system_package_primitives::{
    SystemPackageManager, default_system_package_install_recipes_for_current_host,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::run_recipe;

pub(crate) fn execute_system_package_item(
    item: &InstallPlanItem,
) -> OperationResult<BootstrapItem> {
    let package = item
        .package
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("system_package method requires `package`"))?;
    let recipes = if let Some(manager) = item
        .manager
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    {
        let manager = SystemPackageManager::parse(&manager)
            .ok_or_else(|| OperationError::install(format!("unsupported manager `{manager}`")))?;
        vec![manager.install_recipe(&package)]
    } else {
        default_system_package_install_recipes_for_current_host(&package)
    };
    if recipes.is_empty() {
        return Err(OperationError::install(format!(
            "no available package manager recipe for `{package}`"
        )));
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        match run_recipe(&recipe.program, &recipe.args) {
            Ok(_) => {
                return Ok(BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Installed,
                    source: Some(format!("system:{}", recipe.program)),
                    source_kind: Some(BootstrapSourceKind::SystemPackage),
                    archive_match: None,
                    destination: item.destination.clone(),
                    detail: None,
                    error_code: None,
                    failure_code: None,
                });
            }
            Err(err) => errors.push(format!("{} failed: {err}", recipe.program)),
        }
    }
    Err(OperationError::install(format!(
        "all package manager recipes failed: {}",
        errors.join(" | ")
    )))
}
