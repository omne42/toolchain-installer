use std::path::Path;

use omne_host_info_primitives::executable_suffix_for_target;

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::{
    command_exists, command_path_exists, run_recipe_with_env, run_recipe_with_env_in_dir,
};

pub(crate) fn execute_go_install_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("go") {
        return Err(OperationError::install("go command not found"));
    }
    let package = item
        .package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("go_install method requires `package`"))?;
    let binary_name = item
        .binary_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(item.id.as_str());
    let destination = managed_dir.join(format!(
        "{binary_name}{}",
        executable_suffix_for_target(target_triple)
    ));
    let package_path = Path::new(package);
    let env = vec![("GOBIN".to_string(), managed_dir.display().to_string())];
    let resolved_package = if package_path.exists() {
        let working_directory = if package_path.is_dir() {
            package_path
        } else {
            package_path.parent().ok_or_else(|| {
                OperationError::install(format!(
                    "cannot determine go_install working directory for {}",
                    package_path.display()
                ))
            })?
        };
        let args = vec!["install".to_string(), ".".to_string()];
        run_recipe_with_env_in_dir("go".as_ref(), &args, &env, Some(working_directory))?;
        package_path.display().to_string()
    } else if package.contains('@') {
        package.to_string()
    } else {
        let version = item
            .version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("latest");
        format!("{package}@{version}")
    };

    if !package_path.exists() {
        let args = vec!["install".to_string(), resolved_package.clone()];
        run_recipe_with_env("go".as_ref(), &args, &env)?;
    }

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
