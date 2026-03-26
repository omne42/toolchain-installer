use omne_host_info_primitives::detect_host_platform;
use omne_process_primitives::{HostRecipeRequest, run_host_recipe};
use omne_system_package_primitives::SystemPackageManager;
use omne_system_package_primitives::default_system_package_install_recipes_for_os;

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{SystemPackageMode, SystemPackagePlanItem};

pub(crate) fn execute_system_package_item(
    item: &SystemPackagePlanItem,
) -> OperationResult<BootstrapItem> {
    let recipes = match item.mode {
        SystemPackageMode::AptGet => {
            vec![SystemPackageManager::AptGet.install_recipe(&item.package)]
        }
        SystemPackageMode::Explicit(manager) => vec![manager.install_recipe(&item.package)],
        SystemPackageMode::Auto => detect_host_platform()
            .map(|platform| {
                default_system_package_install_recipes_for_os(
                    platform.operating_system().as_str(),
                    &item.package,
                )
            })
            .unwrap_or_default(),
    };
    if recipes.is_empty() {
        return Err(OperationError::install(format!(
            "no available package manager recipe for `{}`",
            item.package
        )));
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        match run_host_recipe(&HostRecipeRequest::new(
            recipe.program.as_ref(),
            &recipe.args,
        )) {
            Ok(_) => {
                return Ok(BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Installed,
                    source: Some(format!("system:{}", recipe.program)),
                    source_kind: Some(BootstrapSourceKind::SystemPackage),
                    archive_match: None,
                    destination: None,
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
