use crate::contracts::{
    ExecutionRequest, InstallExecutionResult, InstallPlan, OUTPUT_SCHEMA_VERSION,
    build_failed_bootstrap_item,
};
use crate::error::InstallerResult;
use crate::error::OperationError;
use crate::install_plan::install_plan_validation::{
    plan_requires_managed_dir, resolve_requested_target_triple, validate_destination_conflicts,
    validate_plan_structure,
};
use crate::install_plan::item_destination_resolution::{
    allow_leaf_symlink_in_managed_destination, effective_destination_for_item,
    effective_destination_for_item_without_managed_dir, validate_managed_path_boundary,
};
use crate::install_plan::item_method_dispatch::execute_plan_item;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;
use omne_fs_primitives::AdvisoryLockGuard;
use omne_host_info_primitives::detect_host_target_triple;

use super::execution_context::acquire_managed_dir_execution_lock;

pub async fn apply_install_plan(
    plan: &InstallPlan,
    request: &ExecutionRequest,
) -> InstallerResult<InstallExecutionResult> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| crate::error::InstallerError::install("unsupported host platform/arch"))?;
    let target_triple =
        resolve_requested_target_triple(request.target_triple.as_deref(), &host_triple)?;
    let resolved_items = validate_plan_structure(
        plan,
        &host_triple,
        &target_triple,
        request.plan_base_dir.as_deref(),
    )?;
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple);
    if plan_requires_managed_dir(&resolved_items) && managed_dir.is_none() {
        return Err(crate::error::InstallerError::install(
            "cannot resolve managed toolchain directory",
        ));
    }
    let _managed_dir_lock = lock_managed_dir_if_needed(managed_dir.as_deref())?;
    let cfg = InstallerRuntimeConfig::from_execution_request(request);
    let client = reqwest::Client::builder()
        .http1_only()
        .timeout(cfg.download.http_timeout)
        .user_agent("toolchain-installer")
        .build()
        .map_err(|err| {
            crate::error::InstallerError::download(format!("build http client failed: {err}"))
        })?;
    validate_destination_conflicts(&resolved_items, &target_triple, managed_dir.as_deref())?;

    let mut items = Vec::new();
    for item in &resolved_items {
        let destination_path = match managed_dir.as_deref() {
            Some(managed_dir) => effective_destination_for_item(item, &target_triple, managed_dir),
            None => effective_destination_for_item_without_managed_dir(item),
        };
        let destination = destination_path
            .as_ref()
            .map(|path| path.display().to_string());
        let allow_leaf_symlink = allow_leaf_symlink_in_managed_destination(item);
        if let Some(path) = destination_path.as_ref()
            && let Some(managed_dir) = managed_dir.as_deref()
            && let Err(detail) =
                validate_managed_path_boundary(path, managed_dir, allow_leaf_symlink)
        {
            items.push(build_boundary_failure_item(
                item.id(),
                destination.clone(),
                detail,
            ));
            continue;
        }
        let bootstrap_item =
            match execute_plan_item(item, &target_triple, managed_dir.as_deref(), &cfg, &client)
                .await
            {
                Ok(bootstrap_item) => {
                    if allow_leaf_symlink
                        && let Some(path) = destination_path.as_ref()
                        && let Some(managed_dir) = managed_dir.as_deref()
                        && let Err(detail) =
                            validate_managed_path_boundary(path, managed_dir, allow_leaf_symlink)
                    {
                        build_boundary_failure_item(item.id(), destination.clone(), detail)
                    } else {
                        bootstrap_item
                    }
                }
                Err(err) => {
                    let (detail, error_code, exit_code) = err.into_failure_parts();
                    build_failed_bootstrap_item(
                        item.id().to_string(),
                        destination,
                        detail,
                        error_code,
                        exit_code,
                    )
                }
            };
        items.push(bootstrap_item);
    }

    Ok(InstallExecutionResult {
        schema_version: OUTPUT_SCHEMA_VERSION,
        host_triple,
        target_triple,
        managed_dir: managed_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        items,
    })
}

fn lock_managed_dir_if_needed(
    managed_dir: Option<&std::path::Path>,
) -> InstallerResult<Option<AdvisoryLockGuard>> {
    managed_dir
        .map(acquire_managed_dir_execution_lock)
        .transpose()
}

fn build_boundary_failure_item(
    item_id: &str,
    destination: Option<String>,
    detail: String,
) -> crate::contracts::BootstrapItem {
    let err = OperationError::install(detail);
    let (detail, error_code, exit_code) = err.into_failure_parts();
    build_failed_bootstrap_item(
        item_id.to_string(),
        destination,
        detail,
        error_code,
        exit_code,
    )
}
