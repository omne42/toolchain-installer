use std::path::Path;

use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::CargoInstallPlanItem;

use super::item_destination_resolution::resolve_cargo_install_destination;

pub(crate) fn execute_cargo_install_item(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("cargo") {
        return Err(OperationError::install("cargo command not found"));
    }
    let install_root = managed_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| OperationError::install("cannot determine cargo install root"))?;
    let destination = resolve_cargo_install_destination(item, target_triple, managed_dir);

    let mut args = vec!["install".to_string()];
    let package_path = Path::new(&item.package);
    let source = if package_path.exists() {
        args.push("--path".to_string());
        args.push(package_path.display().to_string());
        format!("cargo:path:{}", package_path.display())
    } else {
        args.push("--locked".to_string());
        args.push(item.package.clone());
        if let Some(version) = item.version.as_deref() {
            args.push("--version".to_string());
            args.push(version.to_string());
        }
        format!("cargo:crate:{}", item.package)
    };
    args.push("--root".to_string());
    args.push(install_root.display().to_string());
    run_host_recipe(&HostRecipeRequest::new("cargo".as_ref(), &args))
        .map_err(OperationError::from_host_recipe)?;

    if !command_path_exists(&destination) {
        return Err(OperationError::install(format!(
            "expected cargo_install binary at {}",
            destination.display()
        )));
    }

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(source),
        source_kind: None,
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}
