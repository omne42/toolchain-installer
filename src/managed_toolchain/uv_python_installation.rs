use std::ffi::OsString;
use std::path::Path;

use crate::contracts::BootstrapItem;
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::managed_uv_process_env;
use crate::managed_toolchain::managed_python_executable_discovery::{
    capture_managed_python_installation_state, find_updated_managed_python_executable,
};
use crate::managed_toolchain::managed_uv_host_execution::run_managed_uv_recipe;
use crate::managed_toolchain::managed_uv_installation::{
    ManagedUvBootstrapMode, ensure_managed_uv,
};
use crate::managed_toolchain::source_candidate_attempts::attempt_source_candidates;
use crate::managed_toolchain::uv_installation_source_candidates::{
    prioritize_reachable_installation_sources, python_installation_source_candidates,
};
use crate::plan_items::UvPythonPlanItem;

pub(crate) async fn execute_uv_python_item(
    item: &UvPythonPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let (uv, uv_detail) = ensure_managed_uv(
        target_triple,
        managed_dir,
        cfg,
        client,
        ManagedUvBootstrapMode::Reusable {
            preferred_python: None,
        },
    )
    .await?;
    let base_env = managed_uv_process_env(managed_dir);
    let candidates = prioritize_reachable_installation_sources(
        client,
        python_installation_source_candidates(cfg),
    )
    .await;
    attempt_source_candidates(candidates, "all uv_python sources failed", |candidate| {
        let candidate_label = candidate.label.clone();
        let mut env = base_env.clone();
        env.extend(
            candidate
                .env
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value))),
        );
        let args = vec![
            "python".to_string(),
            "install".to_string(),
            "--force".to_string(),
            item.version.to_string(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let preinstall_state =
            capture_managed_python_installation_state(managed_dir, target_triple);
        run_managed_uv_recipe(uv.program.as_os_str(), &args, &env)
            .map_err(|err| OperationError::install(format!("{candidate_label} failed: {err}")))?;
        let destination = find_updated_managed_python_executable(
            managed_dir,
            &item.version,
            target_triple,
            &preinstall_state,
        )
        .ok_or_else(|| {
            OperationError::install(format!(
                "{} failed: {}",
                candidate_label,
                OperationError::install(format!(
                    "uv python install succeeded but no newly created or updated managed Python executable matching `{}` was found",
                    item.version
                ))
                .detail()
            ))
        })?;
        Ok(build_installed_bootstrap_item(
            &item.id,
            candidate.label,
            candidate.source_kind,
            &destination,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        ))
    })
}
