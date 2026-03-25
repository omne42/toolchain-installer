use std::path::Path;

use crate::contracts::{BootstrapItem, BootstrapSourceKind, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::{
    managed_python_installation_dir, managed_uv_process_env,
};
use crate::managed_toolchain::managed_python_executable_discovery::find_managed_python_executable;
use crate::managed_toolchain::managed_uv_installation::ensure_managed_uv;
use crate::managed_toolchain::source_candidate_attempts::attempt_source_candidates;
use crate::managed_toolchain::uv_installation_source_candidates::python_installation_source_candidates;
use crate::platform::process_runner::run_recipe_with_env;

pub(crate) async fn execute_uv_python_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let version = item
        .version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("uv_python method requires `version`"))?;
    let (uv, uv_detail) = ensure_managed_uv(target_triple, managed_dir, cfg, client).await?;
    let base_env = managed_uv_process_env(managed_dir);
    let candidates = python_installation_source_candidates(cfg);
    attempt_source_candidates(candidates, "all uv_python sources failed", |candidate| {
        let mut env = base_env.clone();
        env.extend(candidate.env.iter().cloned());
        let args = vec![
            "python".to_string(),
            "install".to_string(),
            "--force".to_string(),
            version.to_string(),
        ];
        run_recipe_with_env(uv.program.as_os_str(), &args, &env)
            .map_err(|err| format!("{} failed: {err}", candidate.label.clone()))?;
        let destination = find_managed_python_executable(managed_dir, version, target_triple)
            .unwrap_or_else(|| managed_python_installation_dir(managed_dir));
        Ok(build_installed_bootstrap_item(
            item,
            candidate.label,
            BootstrapSourceKind::PythonMirror,
            &destination,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        ))
    })
}
