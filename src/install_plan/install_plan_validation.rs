use std::collections::HashSet;
use std::path::{Path, PathBuf};

use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};

use crate::contracts::{ExecutionRequest, InstallPlan, PLAN_SCHEMA_VERSION};
use crate::error::{InstallerError, InstallerResult};
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;
use crate::plan_items::ResolvedPlanItem;

use super::item_destination_resolution::effective_destination_for_item;
use super::resolved_plan_item::resolve_plan_item;

pub fn validate_install_plan(
    plan: &InstallPlan,
    requested_target_triple: Option<&str>,
) -> InstallerResult<()> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(requested_target_triple, &host_triple);
    validate_plan_structure(plan, &host_triple, &target_triple, None).map(|_| ())
}

pub fn validate_install_plan_with_request(
    plan: &InstallPlan,
    request: &ExecutionRequest,
) -> InstallerResult<()> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(request.target_triple.as_deref(), &host_triple);
    let resolved_items = validate_plan_structure(
        plan,
        &host_triple,
        &target_triple,
        request.plan_base_dir.as_deref(),
    )?;
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| InstallerError::install("cannot resolve managed toolchain directory"))?;
    validate_destination_conflicts(&resolved_items, &target_triple, &managed_dir)
}

#[cfg(test)]
pub(crate) fn validate_plan(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<Vec<ResolvedPlanItem>> {
    validate_plan_structure(plan, host_triple, target_triple, None)
}

#[cfg(test)]
pub(crate) fn validate_plan_with_base_dir(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: &Path,
) -> InstallerResult<Vec<ResolvedPlanItem>> {
    validate_plan_structure(plan, host_triple, target_triple, Some(plan_base_dir))
}

#[cfg(test)]
pub(crate) fn validate_plan_with_managed_dir(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
    managed_dir: &Path,
) -> InstallerResult<Vec<ResolvedPlanItem>> {
    let resolved_items = validate_plan_structure(plan, host_triple, target_triple, None)?;
    validate_destination_conflicts(&resolved_items, target_triple, managed_dir)?;
    Ok(resolved_items)
}

pub(crate) fn validate_plan_structure(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<Vec<ResolvedPlanItem>> {
    if let Some(schema_version) = plan.schema_version
        && schema_version != PLAN_SCHEMA_VERSION
    {
        return Err(InstallerError::usage(format!(
            "unsupported plan schema_version `{schema_version}`; expected `{PLAN_SCHEMA_VERSION}`"
        )));
    }
    if plan.items.is_empty() {
        return Err(InstallerError::usage(
            "install plan must contain at least one item",
        ));
    }

    let resolved_items = plan
        .items
        .iter()
        .map(|item| resolve_plan_item(item, host_triple, target_triple, plan_base_dir))
        .collect::<InstallerResult<Vec<_>>>()?;
    validate_unique_ids(&resolved_items)?;
    Ok(resolved_items)
}

fn validate_unique_ids(items: &[ResolvedPlanItem]) -> InstallerResult<()> {
    let mut seen = HashSet::new();
    for item in items {
        let id = item.id();
        if !seen.insert(id.to_string()) {
            return Err(InstallerError::usage(format!(
                "install plan contains duplicate item id `{id}`"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_destination_conflicts(
    items: &[ResolvedPlanItem],
    target_triple: &str,
    managed_dir: &Path,
) -> InstallerResult<()> {
    let mut destinations: Vec<(&ResolvedPlanItem, PathBuf, Vec<String>)> = Vec::new();
    for item in items {
        let Some(destination) = effective_destination_for_item(item, target_triple, managed_dir)
        else {
            continue;
        };
        let normalized_destination = normalize_destination_components(&destination, target_triple);
        for (existing_item, existing_destination, existing_normalized) in &destinations {
            if destination_conflict_is_allowed(
                existing_item,
                item,
                existing_normalized,
                &normalized_destination,
            ) {
                continue;
            }
            if *existing_normalized == normalized_destination {
                return Err(InstallerError::usage(format!(
                    "install plan items `{}` and `{}` resolve to the same destination `{}`",
                    existing_item.id(),
                    item.id(),
                    destination.display()
                )));
            }
            if destinations_overlap(existing_normalized, &normalized_destination) {
                return Err(InstallerError::usage(format!(
                    "install plan items `{}` and `{}` resolve to overlapping destinations `{}` and `{}`",
                    existing_item.id(),
                    item.id(),
                    existing_destination.display(),
                    destination.display()
                )));
            }
        }
        destinations.push((item, destination, normalized_destination));
    }
    Ok(())
}

fn destination_conflict_is_allowed(
    existing_item: &ResolvedPlanItem,
    candidate_item: &ResolvedPlanItem,
    existing_destination: &[String],
    candidate_destination: &[String],
) -> bool {
    match (existing_item, candidate_item) {
        (ResolvedPlanItem::UvPython(_), ResolvedPlanItem::UvPython(_)) => true,
        (ResolvedPlanItem::WorkspacePackage(_), ResolvedPlanItem::WorkspacePackage(_)) => {
            existing_destination == candidate_destination
        }
        _ => false,
    }
}

fn normalize_destination_components(path: &Path, target_triple: &str) -> Vec<String> {
    let windows_target = target_triple.contains("windows");
    let case_insensitive_target = windows_target || target_triple.contains("darwin");
    let windows_path;
    let comparable_path = if windows_target {
        windows_path = path.to_string_lossy().replace('\\', "/");
        Path::new(&windows_path)
    } else {
        path
    };

    comparable_path
        .components()
        .filter_map(|component| match component {
            std::path::Component::CurDir => None,
            std::path::Component::RootDir => Some("/".to_string()),
            std::path::Component::Prefix(prefix) => {
                Some(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            std::path::Component::Normal(segment) => {
                let value = segment.to_string_lossy();
                Some(if case_insensitive_target {
                    value.to_ascii_lowercase()
                } else {
                    value.into_owned()
                })
            }
            std::path::Component::ParentDir => Some("..".to_string()),
        })
        .collect()
}

fn destinations_overlap(existing: &[String], candidate: &[String]) -> bool {
    is_component_prefix(candidate, existing) || is_component_prefix(existing, candidate)
}

fn is_component_prefix(prefix: &[String], candidate: &[String]) -> bool {
    candidate.starts_with(prefix)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::contracts::{InstallPlan, InstallPlanItem, PLAN_SCHEMA_VERSION};

    use super::validate_plan_with_managed_dir;

    #[test]
    fn validate_destination_conflicts_rejects_case_only_collisions_on_macos() {
        let plan = InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![
                release_item("ruff-upper", "bin/Ruff"),
                release_item("ruff-lower", "bin/ruff"),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "aarch64-apple-darwin",
            "aarch64-apple-darwin",
            Path::new("/tmp/managed"),
        )
        .expect_err("macOS should reject case-only destination collisions");

        assert!(err.to_string().contains("resolve to the same destination"));
    }

    #[test]
    fn validate_destination_conflicts_rejects_overlapping_default_archive_tree_destination() {
        let plan = InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![
                archive_tree_release_item("python-tree", None),
                release_item("python-bin", "python-tree/bin/python3"),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect_err("default archive_tree_release destination should reserve its directory tree");

        assert!(
            err.to_string()
                .contains("resolve to overlapping destinations")
        );
    }

    #[test]
    fn validate_destination_conflicts_rejects_uv_python_install_root_overlap() {
        let plan = InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![
                uv_python_item("python", "3.13.12"),
                release_item("python-shim", ".uv-python/cpython-3.13.12/bin/python3"),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect_err("uv_python should reserve the managed .uv-python tree");

        assert!(
            err.to_string()
                .contains("resolve to overlapping destinations")
        );
    }

    #[test]
    fn validate_destination_conflicts_rejects_case_only_overlaps_on_windows() {
        let plan = InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![
                archive_tree_release_item("python-tree", Some("Bin/Python")),
                release_item("python-bin", "bin/python/python.exe"),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-pc-windows-msvc",
            "x86_64-pc-windows-msvc",
            Path::new(r"C:\managed"),
        )
        .expect_err("Windows targets should reject case-only overlapping destinations");

        assert!(
            err.to_string()
                .contains("resolve to overlapping destinations")
        );
    }

    #[test]
    fn validate_destination_conflicts_allows_multiple_uv_python_versions() {
        let plan = InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![
                uv_python_item("python-312", "3.12.11"),
                uv_python_item("python-313", "3.13.2"),
            ],
        };

        validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect("different uv_python items should be allowed to share the managed install root");
    }

    #[test]
    fn validate_destination_conflicts_allows_workspace_package_reuse_of_same_workspace() {
        let plan = InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![
                workspace_package_item("eslint", "/tmp/repo"),
                workspace_package_item("prettier", "/tmp/repo"),
            ],
        };

        validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect("workspace_package should allow multiple package installs into one workspace");
    }

    fn release_item(id: &str, destination: &str) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.invalid/tool".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some(destination.to_string()),
            package: None,
            manager: None,
            python: None,
        }
    }

    fn archive_tree_release_item(id: &str, destination: Option<&str>) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "archive_tree_release".to_string(),
            version: None,
            url: Some("https://example.invalid/tool.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: destination.map(str::to_string),
            package: None,
            manager: None,
            python: None,
        }
    }

    fn uv_python_item(id: &str, version: &str) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "uv_python".to_string(),
            version: Some(version.to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        }
    }

    fn workspace_package_item(id: &str, destination: &str) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "workspace_package".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some(destination.to_string()),
            package: Some(id.to_string()),
            manager: Some("npm".to_string()),
            python: None,
        }
    }
}
