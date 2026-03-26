use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};

use crate::contracts::{ExecutionRequest, InstallPlan, PLAN_SCHEMA_VERSION};
use crate::error::{InstallerError, InstallerResult};
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;
use crate::plan_items::ResolvedPlanItem;

use super::item_destination_resolution::effective_destination_for_item;
use super::resolved_plan_item::resolve_plan_item;

const VALIDATION_MANAGED_DIR: &str = "__toolchain_installer_validation__/bin";

pub fn validate_install_plan(
    plan: &InstallPlan,
    requested_target_triple: Option<&str>,
) -> InstallerResult<()> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(requested_target_triple, &host_triple);
    let validation_managed_dir = PathBuf::from(VALIDATION_MANAGED_DIR);
    validate_plan_with_managed_dir(plan, &host_triple, &target_triple, &validation_managed_dir)
        .map(|_| ())
}

pub fn validate_install_plan_with_request(
    plan: &InstallPlan,
    request: &ExecutionRequest,
) -> InstallerResult<()> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(request.target_triple.as_deref(), &host_triple);
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| InstallerError::install("cannot resolve managed toolchain directory"))?;
    validate_plan_with_managed_dir(plan, &host_triple, &target_triple, &managed_dir).map(|_| ())
}

#[cfg(test)]
pub(crate) fn validate_plan(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<Vec<ResolvedPlanItem>> {
    let validation_managed_dir = PathBuf::from(VALIDATION_MANAGED_DIR);
    validate_plan_with_managed_dir(plan, host_triple, target_triple, &validation_managed_dir)
}

pub(crate) fn validate_plan_with_managed_dir(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
    managed_dir: &Path,
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
        .map(|item| resolve_plan_item(item, host_triple, target_triple))
        .collect::<InstallerResult<Vec<_>>>()?;
    validate_unique_ids(&resolved_items)?;
    validate_destination_conflicts(&resolved_items, target_triple, managed_dir)?;
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

fn validate_destination_conflicts(
    items: &[ResolvedPlanItem],
    target_triple: &str,
    managed_dir: &Path,
) -> InstallerResult<()> {
    let mut destinations = HashMap::new();
    for item in items {
        let Some(destination) = effective_destination_for_item(item, target_triple, managed_dir)
        else {
            continue;
        };
        if let Some(existing_id) = destinations.insert(destination.clone(), item.id().to_string()) {
            return Err(InstallerError::usage(format!(
                "install plan items `{existing_id}` and `{}` resolve to the same destination `{}`",
                item.id(),
                destination.display()
            )));
        }
    }
    Ok(())
}
