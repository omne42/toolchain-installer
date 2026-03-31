use crate::contracts::{
    BootstrapResult, ExecutionRequest, InstallPlan, OUTPUT_SCHEMA_VERSION,
    build_failed_bootstrap_item,
};
use crate::error::OperationError;
use crate::error::{InstallerError, InstallerResult};
use crate::install_plan::install_plan_validation::{
    validate_destination_conflicts, validate_plan_structure,
};
use crate::install_plan::item_destination_resolution::{
    allow_leaf_symlink_in_managed_destination, effective_destination_for_item,
    validate_managed_path_boundary,
};
use crate::install_plan::item_method_dispatch::execute_plan_item;
use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};

use super::execution_context::ExecutionContext;

pub async fn apply_install_plan(
    plan: &InstallPlan,
    request: &ExecutionRequest,
) -> InstallerResult<BootstrapResult> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(request.target_triple.as_deref(), &host_triple)
        .map_err(|err| InstallerError::usage(err.to_string()))?;
    let resolved_items = validate_plan_structure(
        plan,
        &host_triple,
        &target_triple,
        request.plan_base_dir.as_deref(),
    )?;
    let ctx = ExecutionContext::for_install_plan(request)?;
    validate_destination_conflicts(&resolved_items, &ctx.target_triple, &ctx.managed_dir)?;

    let mut items = Vec::new();
    for item in &resolved_items {
        let destination_path =
            effective_destination_for_item(item, &ctx.target_triple, &ctx.managed_dir);
        let destination = destination_path
            .as_ref()
            .map(|path| path.display().to_string());
        if let Some(path) = destination_path.as_ref()
            && let Err(detail) = validate_managed_path_boundary(
                path,
                &ctx.managed_dir,
                allow_leaf_symlink_in_managed_destination(item),
            )
        {
            let err = OperationError::install(detail);
            let (detail, error_code, exit_code) = err.into_failure_parts();
            items.push(build_failed_bootstrap_item(
                item.id().to_string(),
                destination,
                detail,
                error_code,
                exit_code,
            ));
            continue;
        }
        let bootstrap_item = match execute_plan_item(
            item,
            &ctx.target_triple,
            &ctx.managed_dir,
            &ctx.cfg,
            &ctx.client,
        )
        .await
        {
            Ok(bootstrap_item) => bootstrap_item,
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

    Ok(BootstrapResult {
        schema_version: OUTPUT_SCHEMA_VERSION,
        host_triple: ctx.host_triple,
        target_triple: ctx.target_triple,
        managed_dir: ctx.managed_dir.display().to_string(),
        items,
    })
}
