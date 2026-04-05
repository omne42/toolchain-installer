use std::collections::HashSet;
use std::path::{Path, PathBuf};

use omne_fs_primitives::filesystem_is_case_sensitive;
use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};

use crate::contracts::{ExecutionRequest, InstallPlan, PLAN_SCHEMA_VERSION};
use crate::error::{InstallerError, InstallerResult};
use crate::managed_toolchain::managed_environment_layout::{
    bootstrap_uv_root, managed_python_installation_dir, managed_python_shim_paths,
    managed_uv_cache_dir, managed_uv_tool_dir,
};
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;
use crate::plan_items::ResolvedPlanItem;

use super::item_destination_resolution::{
    effective_destination_for_item, effective_destination_for_item_without_managed_dir,
    npm_global_internal_state_roots,
};
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
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple);
    if plan_requires_managed_dir(&resolved_items) && managed_dir.is_none() {
        return Err(InstallerError::install(
            "cannot resolve managed toolchain directory",
        ));
    }
    validate_destination_conflicts(&resolved_items, &target_triple, managed_dir.as_deref())
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
    validate_destination_conflicts(&resolved_items, target_triple, Some(managed_dir))?;
    Ok(resolved_items)
}

pub(crate) fn validate_plan_structure(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<Vec<ResolvedPlanItem>> {
    if plan.schema_version != PLAN_SCHEMA_VERSION {
        return Err(InstallerError::usage(format!(
            "unsupported plan schema_version `{}`; expected `{PLAN_SCHEMA_VERSION}`",
            plan.schema_version
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

pub(crate) fn plan_requires_managed_dir(items: &[ResolvedPlanItem]) -> bool {
    items.iter().any(item_requires_managed_dir)
}

fn item_requires_managed_dir(item: &ResolvedPlanItem) -> bool {
    !matches!(
        item,
        ResolvedPlanItem::SystemPackage(_)
            | ResolvedPlanItem::Pip(_)
            | ResolvedPlanItem::WorkspacePackage(_)
            | ResolvedPlanItem::RustupComponent(_)
    )
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
    managed_dir: Option<&Path>,
) -> InstallerResult<()> {
    let mut destinations: Vec<ReservedDestination<'_>> = Vec::new();
    for item in items {
        for reserved in reserved_destinations_for_item(item, target_triple, managed_dir) {
            let normalized_destination =
                normalize_destination_components(&reserved.path, target_triple);
            for existing in &destinations {
                if destination_conflict_is_allowed(
                    existing,
                    &reserved,
                    item,
                    &normalized_destination,
                ) {
                    continue;
                }
                if existing.normalized == normalized_destination {
                    return Err(InstallerError::usage(format!(
                        "install plan items `{}` and `{}` resolve to the same destination `{}`",
                        existing.item.id(),
                        item.id(),
                        reserved.path.display()
                    )));
                }
                if destinations_overlap(&existing.normalized, &normalized_destination) {
                    return Err(InstallerError::usage(format!(
                        "install plan items `{}` and `{}` resolve to overlapping destinations `{}` and `{}`",
                        existing.item.id(),
                        item.id(),
                        existing.path.display(),
                        reserved.path.display()
                    )));
                }
            }
            destinations.push(ReservedDestination {
                item,
                path: reserved.path,
                normalized: normalized_destination,
                sharing: reserved.sharing,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SharedReservation {
    Workspace,
    ManagedPython,
    ManagedUvBootstrap,
    ManagedUvCache,
    ManagedUvTool,
    NpmGlobalState,
}

struct ReservedDestinationSpec {
    path: PathBuf,
    sharing: Option<SharedReservation>,
}

struct ReservedDestination<'a> {
    item: &'a ResolvedPlanItem,
    path: PathBuf,
    normalized: Vec<String>,
    sharing: Option<SharedReservation>,
}

fn reserved_destinations_for_item(
    item: &ResolvedPlanItem,
    target_triple: &str,
    managed_dir: Option<&Path>,
) -> Vec<ReservedDestinationSpec> {
    let mut destinations = managed_dir
        .and_then(|managed_dir| effective_destination_for_item(item, target_triple, managed_dir))
        .or_else(|| effective_destination_for_item_without_managed_dir(item))
        .into_iter()
        .map(|path| ReservedDestinationSpec {
            path,
            sharing: None,
        })
        .collect::<Vec<_>>();
    let Some(managed_dir) = managed_dir else {
        if matches!(item, ResolvedPlanItem::WorkspacePackage(_))
            && let Some(primary) = destinations.first_mut()
        {
            primary.sharing = Some(SharedReservation::Workspace);
        }
        return destinations;
    };
    match item {
        ResolvedPlanItem::UvPython(item) => {
            let managed_python_root = managed_python_installation_dir(managed_dir);
            for destination in &mut destinations {
                if destination.path == managed_python_root {
                    destination.sharing = Some(SharedReservation::ManagedPython);
                }
            }
            destinations.push(ReservedDestinationSpec {
                path: bootstrap_uv_root(managed_dir),
                sharing: Some(SharedReservation::ManagedUvBootstrap),
            });
            destinations.push(ReservedDestinationSpec {
                path: managed_uv_cache_dir(managed_dir),
                sharing: Some(SharedReservation::ManagedUvCache),
            });
            destinations.extend(
                managed_python_shim_paths(&item.version, target_triple, managed_dir)
                    .into_iter()
                    .map(|path| ReservedDestinationSpec {
                        path,
                        sharing: None,
                    }),
            );
        }
        ResolvedPlanItem::UvTool(_) => {
            destinations.extend(
                [
                    (
                        managed_python_installation_dir(managed_dir),
                        SharedReservation::ManagedPython,
                    ),
                    (
                        bootstrap_uv_root(managed_dir),
                        SharedReservation::ManagedUvBootstrap,
                    ),
                    (
                        managed_uv_cache_dir(managed_dir),
                        SharedReservation::ManagedUvCache,
                    ),
                    (
                        managed_uv_tool_dir(managed_dir),
                        SharedReservation::ManagedUvTool,
                    ),
                ]
                .into_iter()
                .map(|(path, sharing)| ReservedDestinationSpec {
                    path,
                    sharing: Some(sharing),
                }),
            );
        }
        ResolvedPlanItem::NpmGlobal(item) => {
            destinations.extend(
                npm_global_internal_state_roots(item, target_triple, managed_dir)
                    .into_iter()
                    .map(|path| ReservedDestinationSpec {
                        path,
                        sharing: Some(SharedReservation::NpmGlobalState),
                    }),
            );
        }
        ResolvedPlanItem::WorkspacePackage(_) => {
            if let Some(primary) = destinations.first_mut() {
                primary.sharing = Some(SharedReservation::Workspace);
            }
        }
        _ => {}
    }
    destinations
}

fn destination_conflict_is_allowed(
    existing: &ReservedDestination<'_>,
    candidate_spec: &ReservedDestinationSpec,
    candidate_item: &ResolvedPlanItem,
    candidate_destination: &[String],
) -> bool {
    if existing
        .sharing
        .zip(candidate_spec.sharing)
        .is_some_and(|(left, right)| {
            left == right
                && shared_reservation_overlap_is_allowed(left, existing.item, candidate_item)
                && existing.normalized == candidate_destination
        })
    {
        return true;
    }

    match (existing.item, candidate_item) {
        (ResolvedPlanItem::UvPython(_), ResolvedPlanItem::UvPython(_)) => true,
        (
            ResolvedPlanItem::WorkspacePackage(existing_item),
            ResolvedPlanItem::WorkspacePackage(candidate_item),
        ) => {
            existing.normalized == candidate_destination
                && existing_item.manager == candidate_item.manager
        }
        _ => false,
    }
}

fn shared_reservation_overlap_is_allowed(
    sharing: SharedReservation,
    existing_item: &ResolvedPlanItem,
    candidate_item: &ResolvedPlanItem,
) -> bool {
    match sharing {
        SharedReservation::Workspace => match (existing_item, candidate_item) {
            (
                ResolvedPlanItem::WorkspacePackage(existing_item),
                ResolvedPlanItem::WorkspacePackage(candidate_item),
            ) => existing_item.manager == candidate_item.manager,
            _ => false,
        },
        SharedReservation::ManagedPython
        | SharedReservation::ManagedUvBootstrap
        | SharedReservation::ManagedUvCache => {
            matches!(
                existing_item,
                ResolvedPlanItem::UvPython(_) | ResolvedPlanItem::UvTool(_)
            ) && matches!(
                candidate_item,
                ResolvedPlanItem::UvPython(_) | ResolvedPlanItem::UvTool(_)
            )
        }
        SharedReservation::ManagedUvTool => {
            matches!(existing_item, ResolvedPlanItem::UvTool(_))
                && matches!(candidate_item, ResolvedPlanItem::UvTool(_))
        }
        SharedReservation::NpmGlobalState => {
            matches!(existing_item, ResolvedPlanItem::NpmGlobal(_))
                && matches!(candidate_item, ResolvedPlanItem::NpmGlobal(_))
        }
    }
}

fn normalize_destination_components(path: &Path, target_triple: &str) -> Vec<String> {
    let windows_target = target_triple.contains("windows");
    let case_insensitive_target = windows_target
        || (target_triple.contains("darwin") && path_uses_case_insensitive_filesystem(path));
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

fn path_uses_case_insensitive_filesystem(path: &Path) -> bool {
    let Some(existing_root) = nearest_existing_directory(path) else {
        return false;
    };

    !filesystem_is_case_sensitive(&existing_root)
}

fn nearest_existing_directory(path: &Path) -> Option<PathBuf> {
    let mut candidate = path.to_path_buf();
    if candidate.extension().is_some() || candidate.file_name().is_some() && !candidate.is_dir() {
        candidate = candidate.parent().unwrap_or(path).to_path_buf();
    }

    loop {
        if candidate.exists() {
            return candidate.is_dir().then_some(candidate);
        }
        let parent = candidate.parent()?;
        candidate = parent.to_path_buf();
    }
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

    use super::{
        path_uses_case_insensitive_filesystem, validate_plan_with_base_dir,
        validate_plan_with_managed_dir,
    };

    #[test]
    fn validate_destination_conflicts_matches_darwin_case_sensitivity_of_host_filesystem() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                release_item("ruff-upper", "bin/Ruff"),
                release_item("ruff-lower", "bin/ruff"),
            ],
        };
        let tmp = tempfile::tempdir().expect("tempdir");
        let managed_dir = tmp.path().join("managed");

        let result = validate_plan_with_managed_dir(
            &plan,
            "aarch64-apple-darwin",
            "aarch64-apple-darwin",
            &managed_dir,
        );

        if path_uses_case_insensitive_filesystem(&managed_dir) {
            let err = result.expect_err(
                "case-insensitive filesystem should reject case-only destination collisions",
            );
            assert!(err.to_string().contains("resolve to the same destination"));
        } else {
            result
                .expect("case-sensitive filesystem should allow case-only destination differences");
        }
    }

    #[test]
    fn validate_destination_conflicts_rejects_overlapping_default_archive_tree_destination() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
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
            schema_version: PLAN_SCHEMA_VERSION,
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
    fn validate_destination_conflicts_rejects_uv_python_top_level_shim_overlap() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                uv_python_item("python", "3.13.12"),
                release_item("python-shim", "python3.13"),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect_err("uv_python should reserve managed top-level python shims");

        assert!(err.to_string().contains("resolve to the same destination"));
    }

    #[test]
    fn validate_destination_conflicts_rejects_uv_tool_internal_state_overlap() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                uv_tool_item("ruff", "ruff"),
                archive_tree_release_item("tool-cache", Some(".uv-tools/ruff")),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect_err("uv_tool should reserve its managed uv state roots");

        assert!(
            err.to_string()
                .contains("resolve to overlapping destinations")
        );
    }

    #[test]
    fn validate_destination_conflicts_allows_uv_python_and_uv_tool_shared_managed_state() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                uv_python_item("python", "3.13.12"),
                uv_tool_item("ruff", "ruff"),
            ],
        };

        validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect("uv_python and uv_tool should be allowed to share managed uv state roots");
    }

    #[test]
    fn validate_destination_conflicts_rejects_npm_global_internal_state_overlap() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                npm_global_item("http-server", "http-server@14.1.1", "npm"),
                archive_tree_release_item(
                    "managed-node-modules",
                    Some("lib/node_modules/http-server"),
                ),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect_err("npm_global should reserve its managed package-state tree");

        assert!(
            err.to_string()
                .contains("resolve to overlapping destinations")
        );
    }

    #[test]
    fn validate_destination_conflicts_allows_multiple_npm_global_items_to_share_state_root() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                npm_global_item("http-server", "http-server@14.1.1", "npm"),
                npm_global_item("prettier", "prettier@3.4.2", "npm"),
            ],
        };

        validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect("npm_global items should be allowed to share their managed package-state root");
    }

    #[test]
    fn validate_destination_conflicts_allows_workspace_package_reuse_with_same_manager() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                workspace_package_item_with_manager(
                    "react",
                    "react@18.3.1",
                    "npm",
                    "/tmp/workspace",
                ),
                workspace_package_item_with_manager("vite", "vite@5.4.0", "npm", "/tmp/workspace"),
            ],
        };

        validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect(
            "workspace_package items should be allowed to reuse a workspace with the same manager",
        );
    }

    #[test]
    fn validate_destination_conflicts_rejects_workspace_package_reuse_with_different_managers() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                workspace_package_item_with_manager(
                    "react",
                    "react@18.3.1",
                    "npm",
                    "/tmp/workspace",
                ),
                workspace_package_item_with_manager("vite", "vite@5.4.0", "pnpm", "/tmp/workspace"),
            ],
        };

        let err = validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect_err(
            "workspace_package should reject reusing a workspace across different managers",
        );

        assert!(err.to_string().contains("resolve to the same destination"));
    }

    #[test]
    fn validate_destination_conflicts_rejects_case_only_overlaps_on_windows() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
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
            schema_version: PLAN_SCHEMA_VERSION,
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
    fn validate_destination_conflicts_allows_shared_uv_state_between_python_and_tools() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                uv_python_item("python", "3.13.12"),
                uv_tool_item("ruff", "ruff"),
                uv_tool_item("mypy", "mypy"),
            ],
        };

        validate_plan_with_managed_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/managed"),
        )
        .expect("uv_python and multiple uv_tool items should share managed uv state");
    }

    #[test]
    fn validate_destination_conflicts_allows_workspace_package_reuse_of_same_workspace() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
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

    #[test]
    fn validate_destination_conflicts_allows_workspace_only_plan_without_managed_dir() {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![workspace_package_item("eslint", "/tmp/repo")],
        };

        let resolved_items = super::validate_plan(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
        )
        .expect("workspace-only plan should resolve");

        super::validate_destination_conflicts(&resolved_items, "x86_64-unknown-linux-gnu", None)
            .expect("workspace-only plan should not require a managed_dir for conflict validation");
    }

    #[test]
    fn validate_destination_conflicts_rejects_workspace_overlap_after_plan_base_dir_normalization()
    {
        let plan = InstallPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            items: vec![
                workspace_package_item("eslint", "repo"),
                workspace_package_item("prettier", "/tmp/root/repo/tools"),
            ],
        };

        let resolved_items = validate_plan_with_base_dir(
            &plan,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Path::new("/tmp/root/plans/../"),
        )
        .expect("plan structure should normalize the plan base directory");

        let err = super::validate_destination_conflicts(
            &resolved_items,
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/tmp/managed")),
        )
        .expect_err("normalized workspace destinations should still participate in overlap checks");

        assert!(
            err.to_string()
                .contains("resolve to overlapping destinations")
        );
    }

    #[test]
    fn validate_plan_structure_requires_schema_version() {
        let err = serde_json::from_str::<InstallPlan>(
            r#"{
  "items": [
    { "id": "eslint", "method": "workspace_package", "package": "eslint", "destination": "/tmp/repo" }
  ]
}"#,
        )
        .expect_err("missing schema_version should be rejected at parse time");

        assert!(err.to_string().contains("schema_version"));
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

    fn uv_tool_item(id: &str, package: &str) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "uv_tool".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some(package.to_string()),
            manager: None,
            python: Some("3.13.12".to_string()),
        }
    }

    fn npm_global_item(id: &str, package: &str, manager: &str) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: Some(id.to_string()),
            destination: None,
            package: Some(package.to_string()),
            manager: Some(manager.to_string()),
            python: None,
        }
    }

    fn workspace_package_item_with_manager(
        id: &str,
        package: &str,
        manager: &str,
        destination: &str,
    ) -> InstallPlanItem {
        InstallPlanItem {
            id: id.to_string(),
            method: "workspace_package".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some(destination.to_string()),
            package: Some(package.to_string()),
            manager: Some(manager.to_string()),
            python: None,
        }
    }
}
