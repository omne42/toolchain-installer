use std::path::Path;

use omne_process_primitives::{HostRecipeRequest, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind};
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::managed_uv_process_env;
use crate::managed_toolchain::managed_python_executable_discovery::find_managed_python_executable;
use crate::managed_toolchain::managed_uv_installation::ensure_managed_uv;
use crate::managed_toolchain::source_candidate_attempts::attempt_source_candidates;
use crate::managed_toolchain::uv_installation_source_candidates::python_installation_source_candidates;
use crate::plan_items::UvPythonPlanItem;

pub(crate) async fn execute_uv_python_item(
    item: &UvPythonPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
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
            item.version.to_string(),
        ];
        run_host_recipe(&HostRecipeRequest::new(uv.program.as_os_str(), &args).with_env(&env))
            .map_err(|err| format!("{} failed: {err}", candidate.label.clone()))?;
        let destination = find_managed_python_executable(managed_dir, &item.version, target_triple)
            .ok_or_else(|| {
                format!(
                    "{} failed: {}",
                    candidate.label,
                    OperationError::install(format!(
                        "uv python install succeeded but no managed Python executable matching `{}` was found",
                        item.version
                    ))
                    .detail()
                )
            })?;
        Ok(build_installed_bootstrap_item(
            &item.id,
            candidate.label,
            BootstrapSourceKind::PythonMirror,
            &destination,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        ))
    })
}
