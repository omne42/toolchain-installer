use std::ffi::OsString;
use std::path::Path;

use crate::contracts::BootstrapItem;
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::ManagedDestinationBackup;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::{
    bootstrap_uv_root, managed_python_installation_dir, managed_python_shim_paths,
    managed_uv_cache_dir, managed_uv_process_env,
};
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
    let uv_python_request =
        managed_python_request(&item.version, target_triple).map_err(OperationError::install)?;
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
            uv_python_request.clone(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let preinstall_state =
            capture_managed_python_installation_state(managed_dir, target_triple);
        let state_backups = ManagedUvPythonStateBackups::stash(
            managed_dir,
            target_triple,
            &item.version,
            &uv.program,
        )
        .map_err(OperationError::install)?;
        if let Err(err) = run_managed_uv_recipe(
            uv.program.as_os_str(),
            &args,
            &env,
            cfg.managed_toolchain.uv_recipe_timeout,
        ) {
            restore_failed_uv_python_install(&state_backups).map_err(OperationError::install)?;
            return Err(OperationError::install(format!(
                "{candidate_label} failed: {err}"
            )));
        }
        let Some(destination) = find_updated_managed_python_executable(
            managed_dir,
            &item.version,
            target_triple,
            &preinstall_state,
        ) else {
            restore_failed_uv_python_install(&state_backups).map_err(OperationError::install)?;
            return Err(OperationError::install(format!(
                "{} failed: {}",
                candidate_label,
                OperationError::install(format!(
                    "uv python install succeeded but no newly created or updated managed Python executable matching `{}` was found",
                    item.version
                ))
                .detail()
            )));
        };
        let detail = build_uv_python_success_detail(
            &state_backups,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        );
        Ok(build_installed_bootstrap_item(
            &item.id,
            candidate.label,
            candidate.source_kind,
            &destination,
            detail,
        ))
    })
}

fn build_uv_python_success_detail(
    state_backups: &ManagedUvPythonStateBackups,
    detail: Option<String>,
) -> Option<String> {
    merge_detail(detail, state_backups.discard_with_warning())
}

fn restore_failed_uv_python_install(
    state_backups: &ManagedUvPythonStateBackups,
) -> Result<(), String> {
    state_backups.restore()
}

struct ManagedUvPythonStateBackups {
    backups: Vec<ManagedDestinationBackup>,
}

impl ManagedUvPythonStateBackups {
    fn stash(
        managed_dir: &Path,
        target_triple: &str,
        version: &str,
        active_uv: &Path,
    ) -> Result<Self, String> {
        let bootstrap_root = bootstrap_uv_root(managed_dir);
        let mut paths = vec![
            (
                managed_python_installation_dir(managed_dir),
                "managed uv python state",
            ),
            (managed_uv_cache_dir(managed_dir), "managed uv cache"),
            (bootstrap_root.clone(), "managed uv bootstrap"),
        ];
        paths.extend(
            managed_python_shim_paths(version, target_triple, managed_dir)
                .into_iter()
                .map(|path| (path, "managed python shim")),
        );
        let backups = paths
            .into_iter()
            .filter(|(path, _)| !path.starts_with(&bootstrap_root) || !active_uv.starts_with(path))
            .map(|(path, label)| ManagedDestinationBackup::stash(&path, label))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { backups })
    }

    fn restore(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        for backup in self.backups.iter().rev() {
            if let Err(err) = backup.restore() {
                errors.push(err);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join(" | "))
        }
    }

    fn discard_with_warning(&self) -> Option<String> {
        self.backups.iter().fold(None, |detail, backup| {
            merge_detail(detail, backup.discard_with_warning())
        })
    }
}

fn merge_detail(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(first), None) => Some(first),
        (None, Some(second)) => Some(second),
        (None, None) => None,
    }
}

pub(super) fn managed_python_request(version: &str, target_triple: &str) -> Result<String, String> {
    let platform = uv_python_platform_selector(target_triple).ok_or_else(|| {
        format!(
            "uv_python does not support managed Python request mapping for target triple `{target_triple}`"
        )
    })?;
    Ok(format!("cpython-{version}-{platform}"))
}

pub(super) fn is_version_like_python_selector(value: &str) -> bool {
    let mut count = 0usize;
    for segment in value.split('.') {
        count += 1;
        if count > 3 || segment.is_empty() || !segment.chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
    }
    count >= 1
}

fn uv_python_platform_selector(target_triple: &str) -> Option<&'static str> {
    match target_triple {
        "x86_64-unknown-linux-gnu" => Some("linux-x86_64-gnu"),
        "aarch64-unknown-linux-gnu" => Some("linux-aarch64-gnu"),
        "x86_64-unknown-linux-musl" => Some("linux-x86_64-musl"),
        "aarch64-unknown-linux-musl" => Some("linux-aarch64-musl"),
        "x86_64-apple-darwin" => Some("macos-x86_64-none"),
        "aarch64-apple-darwin" => Some("macos-aarch64-none"),
        "x86_64-pc-windows-msvc" => Some("windows-x86_64-none"),
        "aarch64-pc-windows-msvc" => Some("windows-aarch64-none"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_version_like_python_selector, managed_python_request, uv_python_platform_selector,
    };

    #[test]
    fn managed_python_request_includes_linux_libc_selector() {
        assert_eq!(
            managed_python_request("3.13.12", "x86_64-unknown-linux-gnu").as_deref(),
            Ok("cpython-3.13.12-linux-x86_64-gnu")
        );
        assert_eq!(
            managed_python_request("3.13.12", "x86_64-unknown-linux-musl").as_deref(),
            Ok("cpython-3.13.12-linux-x86_64-musl")
        );
    }

    #[test]
    fn managed_python_request_maps_supported_non_linux_targets() {
        assert_eq!(
            managed_python_request("3.13", "aarch64-apple-darwin").as_deref(),
            Ok("cpython-3.13-macos-aarch64-none")
        );
        assert_eq!(
            managed_python_request("3", "x86_64-pc-windows-msvc").as_deref(),
            Ok("cpython-3-windows-x86_64-none")
        );
    }

    #[test]
    fn managed_python_request_rejects_unknown_target_triple() {
        let err = managed_python_request("3.13.12", "powerpc64le-unknown-linux-gnu")
            .expect_err("unknown target should fail");
        assert!(err.contains("does not support managed Python request mapping"));
        assert!(uv_python_platform_selector("powerpc64le-unknown-linux-gnu").is_none());
    }

    #[test]
    fn version_like_python_selector_only_accepts_numeric_segments() {
        assert!(is_version_like_python_selector("3"));
        assert!(is_version_like_python_selector("3.13"));
        assert!(is_version_like_python_selector("3.13.12"));
        assert!(!is_version_like_python_selector("python3.13"));
        assert!(!is_version_like_python_selector(
            "cpython-3.13.12-linux-x86_64-gnu"
        ));
        assert!(!is_version_like_python_selector("/tmp/python3.13"));
    }
}
