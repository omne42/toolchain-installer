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
    let mut destinations: Vec<(PathBuf, Vec<String>, String)> = Vec::new();
    for item in items {
        let Some(destination) = effective_destination_for_item(item, target_triple, managed_dir)
        else {
            continue;
        };
        let normalized_destination = normalize_destination_components(&destination, target_triple);
        for (existing_destination, existing_normalized, existing_id) in &destinations {
            if *existing_normalized == normalized_destination {
                return Err(InstallerError::usage(format!(
                    "install plan items `{existing_id}` and `{}` resolve to the same destination `{}`",
                    item.id(),
                    destination.display()
                )));
            }
            if destinations_overlap(existing_normalized, &normalized_destination) {
                return Err(InstallerError::usage(format!(
                    "install plan items `{existing_id}` and `{}` resolve to overlapping destinations `{}` and `{}`",
                    item.id(),
                    existing_destination.display(),
                    destination.display()
                )));
            }
        }
        destinations.push((destination, normalized_destination, item.id().to_string()));
        destinations.push((destination, normalized_destination, item.id().to_string()));
    }
    Ok(())
}

fn normalize_destination_components(path: &Path, target_triple: &str) -> Vec<String> {
    let windows_target = target_triple.contains("windows");
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
                Some(if windows_target {
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
