use std::ffi::OsString;
use std::time::Duration;

use omne_host_info_primitives::detect_host_platform;
use omne_process_primitives::HostRecipeRequest;
use omne_system_package_primitives::{
    SystemPackageName, default_system_package_install_recipes_for_os,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::host_recipe::run_installer_host_recipe;
use crate::installer_runtime_config::DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS;
use crate::plan_items::{SystemPackageMode, SystemPackagePlanItem};

#[allow(dead_code)]
pub(crate) fn execute_system_package_item(
    item: &SystemPackagePlanItem,
) -> OperationResult<BootstrapItem> {
    execute_system_package_item_with_timeout(
        item,
        Duration::from_secs(DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS),
    )
}

pub(crate) fn execute_system_package_item_with_timeout(
    item: &SystemPackagePlanItem,
    timeout: Duration,
) -> OperationResult<BootstrapItem> {
    let package = SystemPackageName::new(&item.package).map_err(|err| {
        OperationError::install(format!(
            "invalid system package name for `{}`: {err}",
            item.package
        ))
    })?;
    let recipes = match item.mode {
        SystemPackageMode::Explicit(manager) => vec![manager.install_recipe(&package)],
        SystemPackageMode::Auto => match detect_host_platform() {
            Some(platform) => default_system_package_install_recipes_for_os(
                platform.operating_system().as_str(),
                &package,
            )
            .map_err(|err| OperationError::install(err.to_string()))?,
            None => Vec::new(),
        },
    };
    if recipes.is_empty() {
        return Err(OperationError::install(format!(
            "no available package manager recipe for `{}`",
            item.package
        )));
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        let args = recipe.args.iter().map(OsString::from).collect::<Vec<_>>();
        match run_installer_host_recipe(
            &HostRecipeRequest::new(recipe.program.as_ref(), &args),
            timeout,
        ) {
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
