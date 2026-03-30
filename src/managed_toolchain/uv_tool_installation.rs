use std::ffi::OsString;
use std::path::Path;

use omne_process_primitives::command_path_exists;

use crate::contracts::{BootstrapItem, BootstrapSourceKind};
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::ManagedDestinationBackup;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::{
    managed_tool_binary_path, managed_uv_process_env,
};
use crate::managed_toolchain::managed_uv_host_execution::run_managed_uv_recipe;
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
        let backup = ManagedDestinationBackup::stash(&destination, "managed binary")?;

        let args = build_uv_tool_install_args(item);

        if let Err(err) = run_managed_uv_recipe(uv.program.as_os_str(), &args, &env) {
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

fn build_uv_tool_install_args(item: &UvToolPlanItem) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("tool"),
        OsString::from("install"),
        OsString::from("--force"),
    ];
    if let Some(python) = item.python.as_deref() {
        args.push(OsString::from("--python"));
        args.push(OsString::from(python));
    }
    if item.binary_name_explicit {
        args.push(OsString::from("--from"));
        args.push(OsString::from(&item.package));
        args.push(OsString::from(&item.binary_name));
    } else {
        args.push(OsString::from(&item.package));
    }
    args
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::build_uv_tool_install_args;
    use crate::plan_items::UvToolPlanItem;

    #[test]
    fn explicit_binary_name_enters_uv_tool_install_args() {
        let item = UvToolPlanItem {
            id: "ruff-installer".to_string(),
            package: "ruff-lsp".to_string(),
            python: Some("3.13".to_string()),
            binary_name: "ruff-lsp".to_string(),
            binary_name_explicit: true,
        };

        let args = build_uv_tool_install_args(&item);
        assert_eq!(
            args,
            vec![
                "tool", "install", "--force", "--python", "3.13", "--from", "ruff-lsp", "ruff-lsp"
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn inferred_binary_name_keeps_plain_uv_tool_install_args() {
        let item = UvToolPlanItem {
            id: "ruff-installer".to_string(),
            package: "ruff-lsp".to_string(),
            python: None,
            binary_name: "ruff-lsp".to_string(),
            binary_name_explicit: false,
        };

        let args = build_uv_tool_install_args(&item);
        assert_eq!(
            args,
            vec!["tool", "install", "--force", "ruff-lsp"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }
}
