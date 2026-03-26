use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

use crate::plan_items::{CargoInstallPlanItem, ReleasePlanItem, ResolvedPlanItem, UvToolPlanItem};

pub(crate) fn effective_destination_for_item(
    item: &ResolvedPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> Option<PathBuf> {
    match item {
        ResolvedPlanItem::Release(item) => Some(resolve_release_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::ArchiveTreeRelease(item) => Some(
            item.destination
                .as_deref()
                .map(|destination| resolve_destination_path(destination, managed_dir))
                .unwrap_or_else(|| managed_dir.join(&item.id)),
        ),
        ResolvedPlanItem::CargoInstall(item) => Some(resolve_cargo_install_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::Uv(_) => {
            Some(managed_dir.join(format!("uv{}", executable_suffix_for_target(target_triple))))
        }
        ResolvedPlanItem::UvTool(item) => Some(resolve_uv_tool_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::WorkspacePackage(item) => {
            Some(resolve_destination_path(&item.destination, managed_dir))
        }
        _ => None,
    }
}

pub(crate) fn resolve_release_destination(
    item: &ReleasePlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    if let Some(destination) = item.destination.as_deref() {
        return resolve_destination_path(destination, managed_dir);
    }
    managed_dir.join(resolve_release_binary_name(item, target_triple))
}

pub(crate) fn resolve_release_binary_name(item: &ReleasePlanItem, target_triple: &str) -> String {
    item.binary_name
        .clone()
        .unwrap_or_else(|| format!("{}{}", item.id, executable_suffix_for_target(target_triple)))
}

pub(crate) fn resolve_cargo_install_destination(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir
        .parent()
        .unwrap_or(managed_dir)
        .join("bin")
        .join(format!(
            "{}{}",
            item.binary_name,
            executable_suffix_for_target(target_triple)
        ))
}

pub(crate) fn resolve_uv_tool_destination(
    item: &UvToolPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ))
}

fn resolve_destination_path(path: &Path, managed_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    managed_dir.join(path)
}
