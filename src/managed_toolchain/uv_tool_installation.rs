use std::ffi::OsString;
use std::path::{Path, PathBuf};

use omne_process_primitives::{HostRecipeRequest, command_path_exists, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind};
use crate::error::OperationResult;
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
use crate::plan_items::UvToolPlanItem;

pub(crate) async fn execute_uv_tool_item(
    item: &UvToolPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let (uv, uv_detail) = ensure_managed_uv(target_triple, managed_dir, cfg, client).await?;
    let base_env = managed_uv_process_env(managed_dir);
    let candidates = prioritize_reachable_installation_sources(
        client,
        package_index_installation_source_candidates(cfg),
    )
    .await;
    let destination = managed_tool_binary_path(&item.binary_name, target_triple, managed_dir);
    attempt_source_candidates(candidates, "all uv_tool sources failed", |candidate| {
        let mut env = base_env
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect::<Vec<_>>();
        env.extend(
            candidate
                .env
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value))),
        );
        let backup = ManagedToolBinaryBackup::stash(&destination)?;

        let mut args = vec![
            "tool".to_string(),
            "install".to_string(),
            "--force".to_string(),
        ];
        if let Some(python) = item.python.as_deref() {
            args.push("--python".to_string());
            args.push(python.to_string());
        }
        args.push(item.package.to_string());
        let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();

        if let Err(err) =
            run_host_recipe(&HostRecipeRequest::new(uv.program.as_os_str(), &args).with_env(&env))
        {
            backup.restore()?;
            return Err(format!("{} failed: {err}", candidate.label.clone()));
        }
        if !command_path_exists(&destination) {
            backup.restore()?;
            return Err(format!(
                "{} installed package `{}` but expected managed binary at {}",
                candidate.label,
                item.package,
                destination.display()
            ));
        }
        backup.discard()?;
        Ok(build_installed_bootstrap_item(
            &item.id,
            candidate.label,
            BootstrapSourceKind::PackageIndex,
            &destination,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        ))
    })
}

struct ManagedToolBinaryBackup {
    original: PathBuf,
    backup: Option<PathBuf>,
}

impl ManagedToolBinaryBackup {
    fn stash(original: &Path) -> Result<Self, String> {
        if !original.exists() {
            return Ok(Self {
                original: original.to_path_buf(),
                backup: None,
            });
        }

        let backup = original.with_file_name(format!(
            "{}.toolchain-installer-backup",
            original
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("managed-tool")
        ));
        if backup.exists() {
            return Err(format!(
                "cannot stage existing managed binary backup {}",
                backup.display()
            ));
        }
        std::fs::rename(original, &backup).map_err(|err| {
            format!(
                "cannot stage existing managed binary {} before reinstall: {err}",
                original.display()
            )
        })?;
        Ok(Self {
            original: original.to_path_buf(),
            backup: Some(backup),
        })
    }

    fn restore(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        if self.original.exists() {
            std::fs::remove_file(&self.original).map_err(|err| {
                format!(
                    "cannot remove failed managed binary {} before restore: {err}",
                    self.original.display()
                )
            })?;
        }
        std::fs::rename(backup, &self.original).map_err(|err| {
            format!(
                "cannot restore previous managed binary {} from {}: {err}",
                self.original.display(),
                backup.display()
            )
        })
    }

    fn discard(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        std::fs::remove_file(backup).map_err(|err| {
            format!(
                "cannot remove staged managed binary backup {}: {err}",
                backup.display()
            )
        })
    }
}
