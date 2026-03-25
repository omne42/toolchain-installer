use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

use crate::contracts::InstallPlanItem;

use super::plan_method::PlanMethod;

pub(crate) fn effective_destination_for_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> Option<PathBuf> {
    match PlanMethod::classify(item).unwrap_or(PlanMethod::Unknown) {
        PlanMethod::Release => Some(resolve_release_destination(
            item,
            target_triple,
            managed_dir,
        )),
        PlanMethod::ArchiveTreeRelease => Some(resolve_archive_tree_destination(item, managed_dir)),
        PlanMethod::CargoInstall => Some(resolve_cargo_install_destination(
            item,
            target_triple,
            managed_dir,
        )),
        PlanMethod::WorkspacePackage => item
            .destination
            .as_deref()
            .map(|raw| resolve_destination_path(raw, managed_dir)),
        _ => item
            .destination
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute()),
    }
}

pub(crate) fn resolve_release_destination(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    let binary_name = item
        .binary_name
        .clone()
        .unwrap_or_else(|| format!("{}{}", item.id, executable_suffix_for_target(target_triple)));
    if let Some(raw_destination) = item.destination.as_deref() {
        return resolve_destination_path(raw_destination, managed_dir);
    }
    managed_dir.join(binary_name)
}

pub(crate) fn resolve_archive_tree_destination(
    item: &InstallPlanItem,
    managed_dir: &Path,
) -> PathBuf {
    if let Some(raw_destination) = item.destination.as_deref() {
        return resolve_destination_path(raw_destination, managed_dir);
    }
    managed_dir.join(&item.id)
}

pub(crate) fn resolve_cargo_install_destination(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    let binary_name = item
        .binary_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(item.id.as_str());
    managed_dir
        .parent()
        .unwrap_or(managed_dir)
        .join("bin")
        .join(format!(
            "{binary_name}{}",
            executable_suffix_for_target(target_triple)
        ))
}

fn resolve_destination_path(raw_destination: &str, managed_dir: &Path) -> PathBuf {
    let path = PathBuf::from(raw_destination.trim());
    if path.is_absolute() {
        return path;
    }
    managed_dir.join(path)
}
