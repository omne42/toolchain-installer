use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};

use super::install_plan_validation::validate_plan;
use super::item_destination_resolution::effective_destination_for_item;
use super::item_method_dispatch::execute_plan_item;
use crate::contracts::{
    BootstrapRequest, BootstrapResult, InstallPlan, OUTPUT_SCHEMA_VERSION,
    build_failed_bootstrap_item,
};
use crate::error::{InstallerError, InstallerResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;

pub async fn apply_install_plan(
    plan: &InstallPlan,
    request: &BootstrapRequest,
) -> InstallerResult<BootstrapResult> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(request.target_triple.as_deref(), &host_triple);
    let resolved_items = validate_plan(plan, &host_triple, &target_triple)?;
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| InstallerError::install("cannot resolve managed toolchain directory"))?;
    let cfg = InstallerRuntimeConfig::from_request(request);
    let client = reqwest::Client::builder()
        // GitHub release asset transfers are more reliable via HTTP/1.1 in our CI/runtime mix.
        .http1_only()
        .timeout(cfg.download.http_timeout)
        .user_agent("toolchain-installer")
        .build()
        .map_err(|err| InstallerError::download(format!("build http client failed: {err}")))?;

    let mut items = Vec::new();
    for item in &resolved_items {
        let destination = effective_destination_for_item(item, &target_triple, &managed_dir)
            .map(|path| path.display().to_string());
        let bootstrap_item =
            match execute_plan_item(item, &target_triple, &managed_dir, &cfg, &client).await {
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
        host_triple,
        target_triple,
        managed_dir: managed_dir.display().to_string(),
        items,
    })
}
