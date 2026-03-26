use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

use crate::managed_toolchain::managed_environment_layout::managed_python_installation_dir;
use crate::plan_items::{
    CargoInstallPlanItem, GoInstallPlanItem, NodePackageManager, NpmGlobalPlanItem,
    ReleasePlanItem, ResolvedPlanItem, UvToolPlanItem,
};

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
        ResolvedPlanItem::NpmGlobal(item) => Some(resolve_npm_global_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::GoInstall(item) => Some(resolve_go_install_destination(
            item,
            target_triple,
            managed_dir,
        )),
        ResolvedPlanItem::Uv(_) => {
            Some(managed_dir.join(format!("uv{}", executable_suffix_for_target(target_triple))))
        }
        ResolvedPlanItem::UvPython(_) => Some(managed_python_installation_dir(managed_dir)),
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

pub(crate) fn resolve_npm_global_destination(
    item: &NpmGlobalPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    match item.manager {
        NodePackageManager::Npm => {
            let prefix_root = if target_triple.contains("windows") {
                managed_dir.to_path_buf()
            } else if managed_dir
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value == "bin")
            {
                managed_dir.parent().unwrap_or(managed_dir).to_path_buf()
            } else {
                managed_dir.to_path_buf()
            };
            if target_triple.contains("windows") {
                prefix_root.join(format!("{}.cmd", item.binary_name))
            } else {
                prefix_root.join("bin").join(&item.binary_name)
            }
        }
        NodePackageManager::Pnpm => managed_dir.join(&item.binary_name),
        NodePackageManager::Bun => managed_dir.join("bin").join(&item.binary_name),
    }
}

pub(crate) fn resolve_go_install_destination(
    item: &GoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir.join(format!(
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
