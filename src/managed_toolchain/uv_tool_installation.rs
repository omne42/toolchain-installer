use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use omne_process_primitives::command_path_exists;

use crate::contracts::{BootstrapItem, BootstrapSourceKind};
use crate::error::OperationResult;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::ManagedDestinationBackup;
use crate::managed_toolchain::bootstrap_item_construction::{
    build_installed_bootstrap_item, build_managed_uv_usage_detail,
};
use crate::managed_toolchain::managed_environment_layout::{
    bootstrap_uv_root, managed_python_installation_dir, managed_tool_binary_path,
    managed_uv_cache_dir, managed_uv_process_env, managed_uv_tool_dir,
};
use crate::managed_toolchain::managed_uv_host_execution::run_managed_uv_recipe;
use crate::managed_toolchain::managed_uv_installation::{
    ManagedUvBootstrapMode, ensure_managed_uv,
};
use crate::managed_toolchain::source_candidate_attempts::attempt_source_candidates;
use crate::managed_toolchain::uv_installation_source_candidates::{
    package_index_installation_source_candidates, prioritize_reachable_installation_sources,
};
use crate::managed_toolchain::version_probe::binary_reports_version;
use crate::plan_items::UvToolPlanItem;

pub(crate) async fn execute_uv_tool_item(
    item: &UvToolPlanItem,
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
            preferred_python: item.python.as_deref(),
        },
    )
    .await?;
    let base_env = managed_uv_process_env(managed_dir);
    let candidates = prioritize_reachable_installation_sources(
        client,
        package_index_installation_source_candidates(cfg),
    )
    .await;
    let destination = managed_tool_binary_path(&item.binary_name, target_triple, managed_dir);
    attempt_source_candidates(candidates, "all uv_tool sources failed", |candidate| {
        let preinstall_state = capture_managed_uv_tool_state(managed_dir);
        let candidate_label = candidate.label.clone();
        let mut env = base_env.clone();
        env.extend(
            candidate
                .env
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value))),
        );
        let state_backups = ManagedUvToolStateBackups::stash(managed_dir, &uv.program)
            .map_err(crate::error::OperationError::install)?;
        let backup = ManagedDestinationBackup::stash(&destination, "managed binary")
            .map_err(crate::error::OperationError::install)?;

        let args = build_uv_tool_install_args(item);

        if let Err(err) = run_managed_uv_recipe(
            uv.program.as_os_str(),
            &args,
            &env,
            cfg.managed_toolchain.uv_recipe_timeout,
        ) {
            restore_failed_uv_tool_install(&backup, &state_backups)
                .map_err(crate::error::OperationError::install)?;
            return Err(crate::error::OperationError::install(format!(
                "{candidate_label} failed: {err}"
            )));
        }
        let Some(installed_destination) = resolve_uv_tool_destination(
            &destination,
            &preinstall_state,
            item,
            target_triple,
            managed_dir,
        ) else {
            restore_failed_uv_tool_install(&backup, &state_backups)
                .map_err(crate::error::OperationError::install)?;
            return Err(crate::error::OperationError::install(format!(
                "{} installed package `{}` but expected managed binary at {}",
                candidate.label,
                item.package,
                destination.display()
            )));
        };
        if !binary_reports_version(&installed_destination) {
            restore_failed_uv_tool_install(&backup, &state_backups)
                .map_err(crate::error::OperationError::install)?;
            return Err(crate::error::OperationError::install(format!(
                "{} installed package `{}` but managed binary at {} failed --version health check",
                candidate.label,
                item.package,
                installed_destination.display()
            )));
        }
        let detail = build_uv_tool_success_detail(
            &backup,
            &state_backups,
            build_managed_uv_usage_detail(&uv.program, uv_detail.clone()),
        );
        Ok(build_installed_bootstrap_item(
            &item.id,
            candidate.label,
            BootstrapSourceKind::PackageIndex,
            &installed_destination,
            detail,
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    modified: Option<SystemTime>,
    len: u64,
}

fn capture_managed_uv_tool_state(managed_dir: &Path) -> HashMap<PathBuf, Option<FileFingerprint>> {
    let mut state = HashMap::new();
    let Ok(entries) = std::fs::read_dir(managed_dir) else {
        return state;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let fingerprint = file_fingerprint(&path);
        state.insert(path, fingerprint);
    }
    state
}

fn resolve_uv_tool_destination(
    expected_destination: &Path,
    preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>,
    item: &UvToolPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> Option<PathBuf> {
    if command_path_exists(expected_destination) {
        return Some(expected_destination.to_path_buf());
    }
    if item.binary_name_explicit {
        return None;
    }

    let mut candidates = discovered_uv_tool_binaries(managed_dir);
    candidates.retain(|path| {
        path_changed(preinstall_state, path)
            && !is_managed_uv_bootstrap_binary(path, target_triple)
            && !is_backup_artifact(path)
    });
    candidates.sort();
    candidates.sort_by_key(|path| {
        (
            !path_file_name_matches(path, &item.id, target_triple),
            !path_file_name_matches(path, &item.binary_name, target_triple),
            path.clone(),
        )
    });
    candidates
        .into_iter()
        .find(|path| command_path_exists(path) && binary_reports_version(path))
}

fn discovered_uv_tool_binaries(managed_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(managed_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                metadata.file_type().is_file() || metadata.file_type().is_symlink()
            })
        })
        .collect()
}

fn is_managed_uv_bootstrap_binary(path: &Path, target_triple: &str) -> bool {
    path.file_name() == managed_tool_binary_path("uv", target_triple, Path::new(".")).file_name()
}

fn is_backup_artifact(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(".toolchain-installer-backup"))
}

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileFingerprint {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

fn path_changed(preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>, path: &Path) -> bool {
    let Some(current) = file_fingerprint(path) else {
        return false;
    };
    match preinstall_state.get(path) {
        Some(Some(previous)) => previous != &current,
        Some(None) | None => true,
    }
}

fn path_file_name_matches(path: &Path, binary_name: &str, target_triple: &str) -> bool {
    let expected = managed_tool_binary_path(binary_name, target_triple, Path::new("."));
    path.file_name() == expected.file_name()
}

fn build_uv_tool_success_detail(
    backup: &ManagedDestinationBackup,
    state_backups: &ManagedUvToolStateBackups,
    detail: Option<String>,
) -> Option<String> {
    merge_detail(
        merge_detail(detail, state_backups.discard_with_warning()),
        backup.discard_with_warning(),
    )
}

fn restore_failed_uv_tool_install(
    backup: &ManagedDestinationBackup,
    state_backups: &ManagedUvToolStateBackups,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(err) = backup.restore() {
        errors.push(err);
    }
    if let Err(err) = state_backups.restore() {
        errors.push(err);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

struct ManagedUvToolStateBackups {
    backups: Vec<ManagedDestinationBackup>,
}

impl ManagedUvToolStateBackups {
    fn stash(managed_dir: &Path, active_uv: &Path) -> Result<Self, String> {
        let bootstrap_root = bootstrap_uv_root(managed_dir);
        let backups = [
            (managed_uv_tool_dir(managed_dir), "managed uv tool state"),
            (managed_uv_cache_dir(managed_dir), "managed uv cache"),
            (bootstrap_root.clone(), "managed uv bootstrap"),
            (
                managed_python_installation_dir(managed_dir),
                "managed uv python state",
            ),
        ]
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

    use super::{
        build_uv_tool_install_args, capture_managed_uv_tool_state, merge_detail,
        resolve_uv_tool_destination,
    };
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

    #[test]
    fn merge_detail_appends_cleanup_warning() {
        assert_eq!(
            merge_detail(
                Some("managed uv detail".to_string()),
                Some("managed binary installed at /tmp/demo but cleanup warning: stale backup remains".to_string()),
            ),
            Some(
                "managed uv detail; managed binary installed at /tmp/demo but cleanup warning: stale backup remains"
                    .to_string()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_uv_tool_destination_prefers_new_binary_matching_item_id() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path();
        let preinstall = capture_managed_uv_tool_state(managed_dir);
        let http = managed_dir.join("http");
        let https = managed_dir.join("https");
        std::fs::write(&http, "#!/bin/sh\necho http 1.0\n").expect("write http");
        std::fs::write(&https, "#!/bin/sh\necho https 1.0\n").expect("write https");
        std::fs::set_permissions(&http, std::fs::Permissions::from_mode(0o755))
            .expect("chmod http");
        std::fs::set_permissions(&https, std::fs::Permissions::from_mode(0o755))
            .expect("chmod https");

        let item = UvToolPlanItem {
            id: "http".to_string(),
            package: "httpie".to_string(),
            python: None,
            binary_name: "httpie".to_string(),
            binary_name_explicit: false,
        };
        let expected = managed_dir.join("httpie");

        let resolved = resolve_uv_tool_destination(
            &expected,
            &preinstall,
            &item,
            "x86_64-unknown-linux-gnu",
            managed_dir,
        )
        .expect("resolved uv tool destination");

        assert_eq!(resolved, http);
    }
}
