use crate::contracts::{
    BootstrapResult, ExecutionRequest, InstallPlan, OUTPUT_SCHEMA_VERSION,
    build_failed_bootstrap_item,
};
use crate::error::InstallerResult;
use crate::install_plan::install_plan_validation::validate_plan_with_managed_dir;
use crate::install_plan::item_destination_resolution::effective_destination_for_item;
use crate::install_plan::item_method_dispatch::execute_plan_item;

use super::execution_context::ExecutionContext;

pub async fn apply_install_plan(
    plan: &InstallPlan,
    request: &ExecutionRequest,
) -> InstallerResult<BootstrapResult> {
    let ctx = ExecutionContext::for_install_plan(request)?;
    let resolved_items = validate_plan_with_managed_dir(
        plan,
        &ctx.host_triple,
        &ctx.target_triple,
        &ctx.managed_dir,
    )?;

    let mut items = Vec::new();
    for item in &resolved_items {
        let destination =
            effective_destination_for_item(item, &ctx.target_triple, &ctx.managed_dir)
                .map(|path| path.display().to_string());
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
