use std::path::Path;

use crate::contracts::{BootstrapItem, BootstrapSourceKind, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::{
    managed_tool_binary_path, managed_uv_process_env,
};
use crate::managed_toolchain::managed_uv_installation::ensure_managed_uv;
use crate::managed_toolchain::source_candidate_attempts::attempt_source_candidates;
use crate::managed_toolchain::uv_installation_source_candidates::{
    package_index_installation_source_candidates, prioritize_reachable_installation_sources,
};
use crate::platform::process_runner::run_recipe_with_env;

pub(crate) async fn execute_uv_tool_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let package = item
        .package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("uv_tool method requires `package`"))?;
    let (uv, uv_detail) = ensure_managed_uv(target_triple, managed_dir, cfg, client).await?;
    let base_env = managed_uv_process_env(managed_dir);
    let candidates = prioritize_reachable_installation_sources(
        client,
        package_index_installation_source_candidates(cfg),
    )
    .await;
    let executable_name = item.id.trim();
    let destination = managed_tool_binary_path(executable_name, target_triple, managed_dir);
    attempt_source_candidates(candidates, "all uv_tool sources failed", |candidate| {
        let mut env = base_env.clone();
        env.extend(candidate.env.iter().cloned());

        let mut args = vec![
            "tool".to_string(),
            "install".to_string(),
            "--force".to_string(),
        ];
        if let Some(python) = item
            .python
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            args.push("--python".to_string());
            args.push(python.to_string());
        }
        args.push(package.to_string());

        run_recipe_with_env(uv.program.as_os_str(), &args, &env)
            .map_err(|err| format!("{} failed: {err}", candidate.label.clone()))?;
        Ok(build_installed_bootstrap_item(
            item,
            candidate.label,
            BootstrapSourceKind::PackageIndex,
            &destination,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        ))
    })
}
