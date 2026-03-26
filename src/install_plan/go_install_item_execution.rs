use std::path::Path;

use omne_host_info_primitives::executable_suffix_for_target;
use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{GoInstallPlanItem, GoInstallSource};

pub(crate) fn execute_go_install_item(
    item: &GoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("go") {
        return Err(OperationError::install("go command not found"));
    }
    let destination = managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ));
    let env = vec![("GOBIN".to_string(), managed_dir.display().to_string())];
    let resolved_package = match &item.source {
        GoInstallSource::LocalPath(package_path) => {
            let working_directory = if package_path.is_dir() {
                package_path.as_path()
            } else {
                package_path.parent().ok_or_else(|| {
                    OperationError::install(format!(
                        "cannot determine go_install working directory for {}",
                        package_path.display()
                    ))
                })?
            };
            let args = vec!["install".to_string(), ".".to_string()];
            run_host_recipe(
                &HostRecipeRequest::new("go".as_ref(), &args)
                    .with_env(&env)
                    .with_working_directory(working_directory),
            )
            .map_err(OperationError::from_host_recipe)?;
            package_path.display().to_string()
        }
        GoInstallSource::PackageSpec(package) => {
            let args = vec!["install".to_string(), package.clone()];
            run_host_recipe(&HostRecipeRequest::new("go".as_ref(), &args).with_env(&env))
                .map_err(OperationError::from_host_recipe)?;
            package.clone()
        }
    };

    if !command_path_exists(&destination) {
        return Err(OperationError::install(format!(
            "expected go_install binary at {}",
            destination.display()
        )));
    }

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("go:install:{resolved_package}")),
        source_kind: None,
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}
