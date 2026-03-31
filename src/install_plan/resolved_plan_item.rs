use std::path::{Path, PathBuf};

use omne_artifact_install_primitives::is_archive_tree_asset_name;
use omne_integrity_primitives::{Sha256Digest, parse_sha256_user_input};
use omne_system_package_primitives::SystemPackageManager;
use reqwest::Url;

use crate::contracts::InstallPlanItem;
use crate::error::{InstallerError, InstallerResult};
use crate::plan_items::{
    ArchiveTreeReleasePlanItem, CargoInstallPlanItem, CargoInstallSource, GoInstallPlanItem,
    GoInstallSource, ManagedUvPlanItem, NodePackageManager, NpmGlobalPlanItem, PipPlanItem,
    ReleasePlanItem, ResolvedPlanItem, RustupComponentPlanItem, SystemPackageMode,
    SystemPackagePlanItem, UvPythonPlanItem, UvToolPlanItem, WorkspacePackagePlanItem,
};

use super::item_destination_resolution::{
    resolve_plan_relative_path, validate_destination, validate_workspace_destination,
};
use super::plan_method::{
    ManagedToolchainMethod, PlanMethod, SUPPORTED_PLAN_METHODS, normalize_plan_method,
};

pub(crate) fn resolve_plan_item(
    item: &InstallPlanItem,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<ResolvedPlanItem> {
    let id = require_plan_item_id(item.id.as_str())?;

    let Some(normalized_method) = normalize_plan_method(&item.method) else {
        return Err(InstallerError::usage(format!(
            "plan item `{id}` has an empty `method`"
        )));
    };
    let method = PlanMethod::from_normalized(&normalized_method);
    if matches!(method, PlanMethod::Unknown) {
        return Err(InstallerError::usage(format!(
            "plan item `{id}` uses unsupported method `{normalized_method}`; supported methods: {}",
            SUPPORTED_PLAN_METHODS.join(", ")
        )));
    }

    if target_triple != host_triple && method.is_host_bound() {
        return Err(InstallerError::usage(format!(
            "plan item `{id}` uses host-bound method `{normalized_method}` but target triple `{target_triple}` does not match host triple `{host_triple}`"
        )));
    }

    match method {
        PlanMethod::Release => resolve_release_plan_item(item, id, host_triple, target_triple),
        PlanMethod::ArchiveTreeRelease => {
            resolve_archive_tree_release_plan_item(item, id, host_triple, target_triple)
        }
        PlanMethod::SystemPackage => resolve_system_package_plan_item(item, id),
        PlanMethod::Apt => resolve_apt_plan_item(item, id),
        PlanMethod::Pip => resolve_pip_plan_item(item, id),
        PlanMethod::NpmGlobal => resolve_npm_global_plan_item(item, id),
        PlanMethod::WorkspacePackage => {
            resolve_workspace_package_plan_item(item, id, host_triple, target_triple, plan_base_dir)
        }
        PlanMethod::CargoInstall => {
            resolve_cargo_install_plan_item(item, id, target_triple, plan_base_dir)
        }
        PlanMethod::RustupComponent => resolve_rustup_component_plan_item(item, id),
        PlanMethod::GoInstall => {
            resolve_go_install_plan_item(item, id, target_triple, plan_base_dir)
        }
        PlanMethod::ManagedToolchain(method) => {
            resolve_managed_toolchain_plan_item(item, id, target_triple, method)
        }
        PlanMethod::Unknown => unreachable!("unsupported method should fail before resolve"),
    }
}

fn resolve_release_plan_item(
    item: &InstallPlanItem,
    id: String,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("package", item.package.as_deref()),
            ("manager", item.manager.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let url = require_http_url(&id, "release", item.url.as_deref())?;
    let sha256 = parse_optional_sha256(&id, item.sha256.as_deref())?;
    let destination =
        parse_optional_destination(&id, item.destination.as_deref(), host_triple, target_triple)?;
    let binary_name = parse_optional_binary_name(&id, item.binary_name.as_deref())?;
    Ok(ResolvedPlanItem::Release(ReleasePlanItem {
        id,
        url,
        sha256,
        archive_binary: optional_trimmed_owned(item.archive_binary.as_deref()),
        binary_name,
        destination,
    }))
}

fn resolve_archive_tree_release_plan_item(
    item: &InstallPlanItem,
    id: String,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("binary_name", item.binary_name.as_deref()),
            ("package", item.package.as_deref()),
            ("manager", item.manager.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let url = require_http_url(&id, "archive_tree_release", item.url.as_deref())?;
    validate_archive_tree_release_asset_name(&id, &url)?;
    let sha256 = parse_optional_sha256(&id, item.sha256.as_deref())?;
    let destination =
        parse_optional_destination(&id, item.destination.as_deref(), host_triple, target_triple)?;
    Ok(ResolvedPlanItem::ArchiveTreeRelease(
        ArchiveTreeReleasePlanItem {
            id,
            url,
            sha256,
            destination,
        },
    ))
}

fn resolve_system_package_plan_item(
    item: &InstallPlanItem,
    id: String,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("binary_name", item.binary_name.as_deref()),
            ("destination", item.destination.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;

    let mode = if let Some(manager) = optional_trimmed(item.manager.as_deref()) {
        let manager = SystemPackageManager::parse(manager).ok_or_else(|| {
            InstallerError::usage(format!(
                "plan item `{id}` uses unsupported manager `{manager}`"
            ))
        })?;
        SystemPackageMode::Explicit(manager)
    } else {
        SystemPackageMode::Auto
    };

    Ok(ResolvedPlanItem::SystemPackage(SystemPackagePlanItem {
        id,
        package: require_non_empty(
            item.package.as_deref().unwrap_or_default(),
            "package",
            item.id.as_str(),
        )?,
        mode,
    }))
}

fn resolve_apt_plan_item(item: &InstallPlanItem, id: String) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("binary_name", item.binary_name.as_deref()),
            ("destination", item.destination.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    if let Some(manager) = optional_trimmed(item.manager.as_deref())
        && manager != "apt-get"
    {
        return Err(InstallerError::usage(format!(
            "plan item `{id}` uses method `apt` but manager `{manager}`"
        )));
    }
    Ok(ResolvedPlanItem::SystemPackage(SystemPackagePlanItem {
        id,
        package: require_non_empty(
            item.package.as_deref().unwrap_or_default(),
            "package",
            item.id.as_str(),
        )?,
        mode: SystemPackageMode::AptGet,
    }))
}

fn resolve_pip_plan_item(item: &InstallPlanItem, id: String) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("binary_name", item.binary_name.as_deref()),
            ("destination", item.destination.as_deref()),
            ("manager", item.manager.as_deref()),
        ],
    )?;
    Ok(ResolvedPlanItem::Pip(PipPlanItem {
        id,
        package: require_non_empty(
            item.package.as_deref().unwrap_or_default(),
            "package",
            item.id.as_str(),
        )?,
        python: optional_trimmed_owned(item.python.as_deref()),
    }))
}

fn resolve_npm_global_plan_item(
    item: &InstallPlanItem,
    id: String,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("destination", item.destination.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let package = require_non_empty(
        item.package.as_deref().unwrap_or_default(),
        "package",
        item.id.as_str(),
    )?;
    let version = optional_trimmed(item.version.as_deref());
    let package_spec = build_versioned_package_spec(&package, version);
    Ok(ResolvedPlanItem::NpmGlobal(NpmGlobalPlanItem {
        package_spec,
        manager: parse_node_package_manager(&id, item.manager.as_deref(), "npm_global")?,
        binary_name: parse_optional_binary_name(&id, item.binary_name.as_deref())?
            .or_else(|| default_binary_name_for_node_package(&package))
            .unwrap_or_else(|| id.clone()),
        id,
    }))
}

fn resolve_workspace_package_plan_item(
    item: &InstallPlanItem,
    id: String,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("binary_name", item.binary_name.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let package = require_non_empty(
        item.package.as_deref().unwrap_or_default(),
        "package",
        item.id.as_str(),
    )?;
    Ok(ResolvedPlanItem::WorkspacePackage(
        WorkspacePackagePlanItem {
            package_spec: package,
            manager: parse_node_package_manager(&id, item.manager.as_deref(), "workspace_package")?,
            destination: require_workspace_destination(
                &id,
                item.destination.as_deref(),
                host_triple,
                target_triple,
                plan_base_dir,
            )?,
            id,
        },
    ))
}

fn resolve_cargo_install_plan_item(
    item: &InstallPlanItem,
    id: String,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("destination", item.destination.as_deref()),
            ("manager", item.manager.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let package = require_non_empty(
        item.package.as_deref().unwrap_or_default(),
        "package",
        item.id.as_str(),
    )?;
    let version = optional_trimmed(item.version.as_deref());
    let source = resolve_cargo_install_source(&package, version, plan_base_dir);
    let binary_name_explicit = optional_trimmed(item.binary_name.as_deref()).is_some();
    Ok(ResolvedPlanItem::CargoInstall(CargoInstallPlanItem {
        binary_name: parse_optional_binary_name(&id, item.binary_name.as_deref())?
            .or_else(|| default_binary_name_for_cargo_source(&source))
            .unwrap_or_else(|| default_binary_name_for_target(target_triple, &id)),
        binary_name_explicit,
        id,
        source,
    }))
}

fn resolve_rustup_component_plan_item(
    item: &InstallPlanItem,
    id: String,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("version", item.version.as_deref()),
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("destination", item.destination.as_deref()),
            ("manager", item.manager.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let binary_name = parse_optional_binary_name(&id, item.binary_name.as_deref())?;
    Ok(ResolvedPlanItem::RustupComponent(RustupComponentPlanItem {
        id,
        component: require_non_empty(
            item.package.as_deref().unwrap_or_default(),
            "package",
            item.id.as_str(),
        )?,
        binary_name,
    }))
}

fn resolve_go_install_plan_item(
    item: &InstallPlanItem,
    id: String,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
            ("url", item.url.as_deref()),
            ("sha256", item.sha256.as_deref()),
            ("archive_binary", item.archive_binary.as_deref()),
            ("destination", item.destination.as_deref()),
            ("manager", item.manager.as_deref()),
            ("python", item.python.as_deref()),
        ],
    )?;
    let package = require_non_empty(
        item.package.as_deref().unwrap_or_default(),
        "package",
        item.id.as_str(),
    )?;
    let source = if looks_like_explicit_go_local_path(&package) {
        GoInstallSource::LocalPath(resolve_plan_relative_path(
            Path::new(&package),
            plan_base_dir,
        ))
    } else if package.contains('@') {
        GoInstallSource::PackageSpec(package)
    } else {
        let version = optional_trimmed(item.version.as_deref()).unwrap_or("latest");
        GoInstallSource::PackageSpec(format!("{package}@{version}"))
    };
    let binary_name = parse_optional_binary_name(&id, item.binary_name.as_deref())?
        .or_else(|| default_binary_name_for_go_source(&source))
        .unwrap_or_else(|| default_binary_name_for_target(target_triple, &id));
    Ok(ResolvedPlanItem::GoInstall(GoInstallPlanItem {
        binary_name,
        id,
        source,
    }))
}

fn resolve_managed_toolchain_plan_item(
    item: &InstallPlanItem,
    id: String,
    target_triple: &str,
    method: ManagedToolchainMethod,
) -> InstallerResult<ResolvedPlanItem> {
    match method {
        ManagedToolchainMethod::Uv => {
            reject_disallowed_fields(
                &id,
                &[
                    ("version", item.version.as_deref()),
                    ("url", item.url.as_deref()),
                    ("sha256", item.sha256.as_deref()),
                    ("archive_binary", item.archive_binary.as_deref()),
                    ("binary_name", item.binary_name.as_deref()),
                    ("destination", item.destination.as_deref()),
                    ("package", item.package.as_deref()),
                    ("manager", item.manager.as_deref()),
                    ("python", item.python.as_deref()),
                ],
            )?;
            Ok(ResolvedPlanItem::Uv(ManagedUvPlanItem { id }))
        }
        ManagedToolchainMethod::UvPython => {
            reject_disallowed_fields(
                &id,
                &[
                    ("url", item.url.as_deref()),
                    ("sha256", item.sha256.as_deref()),
                    ("archive_binary", item.archive_binary.as_deref()),
                    ("binary_name", item.binary_name.as_deref()),
                    ("destination", item.destination.as_deref()),
                    ("package", item.package.as_deref()),
                    ("manager", item.manager.as_deref()),
                    ("python", item.python.as_deref()),
                ],
            )?;
            Ok(ResolvedPlanItem::UvPython(UvPythonPlanItem {
                version: require_uv_python_version(item.id.as_str(), item.version.as_deref())?,
                id,
            }))
        }
        ManagedToolchainMethod::UvTool => {
            reject_disallowed_fields(
                &id,
                &[
                    ("version", item.version.as_deref()),
                    ("url", item.url.as_deref()),
                    ("sha256", item.sha256.as_deref()),
                    ("archive_binary", item.archive_binary.as_deref()),
                    ("destination", item.destination.as_deref()),
                    ("manager", item.manager.as_deref()),
                ],
            )?;
            let binary_name_explicit = optional_trimmed(item.binary_name.as_deref()).is_some();
            Ok(ResolvedPlanItem::UvTool(UvToolPlanItem {
                package: require_non_empty(
                    item.package.as_deref().unwrap_or_default(),
                    "package",
                    item.id.as_str(),
                )?,
                python: optional_trimmed_owned(item.python.as_deref()),
                binary_name: parse_optional_binary_name(&id, item.binary_name.as_deref())?
                    .or_else(|| {
                        item.package
                            .as_deref()
                            .and_then(default_binary_name_for_python_package)
                    })
                    .unwrap_or_else(|| default_binary_name_for_target(target_triple, &id)),
                binary_name_explicit,
                id,
            }))
        }
    }
}

fn parse_node_package_manager(
    item_id: &str,
    raw_manager: Option<&str>,
    method: &str,
) -> InstallerResult<NodePackageManager> {
    match optional_trimmed(raw_manager) {
        None => Ok(NodePackageManager::Npm),
        Some("npm") => Ok(NodePackageManager::Npm),
        Some("pnpm") => Ok(NodePackageManager::Pnpm),
        Some("bun") => Ok(NodePackageManager::Bun),
        Some(value) => Err(InstallerError::usage(format!(
            "plan item `{item_id}` uses unsupported {method} manager `{value}`"
        ))),
    }
}

fn build_versioned_package_spec(package: &str, version: Option<&str>) -> String {
    if let Some(version) = version {
        if node_package_spec_has_embedded_version(package)
            || node_package_spec_uses_explicit_source(package)
        {
            return package.to_string();
        }
        return format!("{package}@{version}");
    }
    package.to_string()
}

fn resolve_cargo_install_source(
    package: &str,
    version: Option<&str>,
    plan_base_dir: Option<&Path>,
) -> CargoInstallSource {
    if looks_like_explicit_cargo_local_path(package) {
        return CargoInstallSource::LocalPath(resolve_plan_relative_path(
            Path::new(package),
            plan_base_dir,
        ));
    }
    CargoInstallSource::RegistryPackage {
        package: package.to_string(),
        version: version.map(ToString::to_string),
    }
}

fn looks_like_explicit_cargo_local_path(package: &str) -> bool {
    package.starts_with('.')
        || package.starts_with('/')
        || package.starts_with('\\')
        || package.contains('/')
        || package.contains('\\')
        || looks_like_windows_drive_path(package)
}

fn looks_like_explicit_go_local_path(package: &str) -> bool {
    package == "."
        || package == ".."
        || package.starts_with("./")
        || package.starts_with(".\\")
        || package.starts_with("../")
        || package.starts_with("..\\")
        || package.starts_with('/')
        || package.starts_with('\\')
        || looks_like_bare_relative_go_local_path(package)
        || looks_like_windows_drive_path(package)
}

fn looks_like_bare_relative_go_local_path(package: &str) -> bool {
    if package.contains('@') || package.contains('\\') {
        return false;
    }

    let Some((first_segment, _)) = package.split_once('/') else {
        return false;
    };
    !first_segment.is_empty() && !first_segment.contains('.') && !first_segment.contains(':')
}

fn looks_like_windows_drive_path(package: &str) -> bool {
    let bytes = package.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn node_package_spec_has_embedded_version(package: &str) -> bool {
    package
        .rfind('@')
        .is_some_and(|index| index > 0 && index + 1 < package.len())
}

fn node_package_spec_uses_explicit_source(package: &str) -> bool {
    [
        "file:",
        "git:",
        "git+",
        "http://",
        "https://",
        "github:",
        "workspace:",
        "link:",
        "npm:",
    ]
    .iter()
    .any(|prefix| package.starts_with(prefix))
}

fn require_http_url(item_id: &str, method: &str, raw_url: Option<&str>) -> InstallerResult<Url> {
    let url = optional_trimmed(raw_url).ok_or_else(|| {
        InstallerError::usage(format!(
            "plan item `{item_id}` with method `{method}` requires `url`"
        ))
    })?;
    let parsed = Url::parse(url).map_err(|err| {
        InstallerError::usage(format!("plan item `{item_id}` has invalid `url`: {err}"))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(InstallerError::usage(format!(
            "plan item `{item_id}` uses unsupported url scheme `{other}`"
        ))),
    }
}

fn validate_archive_tree_release_asset_name(item_id: &str, url: &Url) -> InstallerResult<()> {
    let asset_name = archive_tree_asset_name_from_url(url).ok_or_else(|| {
        InstallerError::usage(format!(
            "plan item `{item_id}` with method `archive_tree_release` requires a URL path ending in a supported archive asset"
        ))
    })?;
    if is_archive_tree_asset_name(asset_name) {
        return Ok(());
    }
    Err(InstallerError::usage(format!(
        "plan item `{item_id}` with method `archive_tree_release` requires a supported archive asset, got `{asset_name}`"
    )))
}

fn archive_tree_asset_name_from_url(url: &Url) -> Option<&str> {
    url.path_segments()?.rfind(|segment| !segment.is_empty())
}

fn parse_optional_sha256(
    item_id: &str,
    raw_sha256: Option<&str>,
) -> InstallerResult<Option<Sha256Digest>> {
    optional_trimmed(raw_sha256)
        .map(|raw| {
            parse_sha256_user_input(raw).ok_or_else(|| {
                InstallerError::usage(format!("plan item `{item_id}` has invalid `sha256` value"))
            })
        })
        .transpose()
}

fn parse_optional_destination(
    item_id: &str,
    raw_destination: Option<&str>,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<Option<PathBuf>> {
    optional_trimmed(raw_destination)
        .map(|destination| validate_destination(item_id, destination, host_triple, target_triple))
        .transpose()
}

fn require_workspace_destination(
    item_id: &str,
    raw_destination: Option<&str>,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<PathBuf> {
    optional_trimmed(raw_destination)
        .map(|destination| {
            validate_workspace_destination(item_id, destination, host_triple, target_triple)
        })
        .transpose()?
        .map(|destination| resolve_plan_relative_path(&destination, plan_base_dir))
        .ok_or_else(|| {
            InstallerError::usage(format!(
                "plan item `{item_id}` with method `workspace_package` requires `destination`"
            ))
        })
}

fn require_plan_item_id(raw: &str) -> InstallerResult<String> {
    let id = require_non_empty(raw, "id", "plan item")?;
    validate_leaf_name(&id, "id", &id)?;
    Ok(id)
}

fn parse_optional_binary_name(
    item_id: &str,
    raw_binary_name: Option<&str>,
) -> InstallerResult<Option<String>> {
    optional_trimmed(raw_binary_name)
        .map(|binary_name| {
            validate_leaf_name(item_id, "binary_name", binary_name)?;
            Ok(binary_name.to_string())
        })
        .transpose()
}

fn validate_leaf_name(item_id: &str, field_name: &str, value: &str) -> InstallerResult<()> {
    if value == "." || value == ".." || looks_like_windows_drive_path(value) {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` `{field_name}` must be a plain file name, got `{value}`"
        )));
    }
    if value.split(['/', '\\']).count() != 1 {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` `{field_name}` must not contain path separators"
        )));
    }
    Ok(())
}

fn default_binary_name_for_target(_target_triple: &str, fallback: &str) -> String {
    fallback.to_string()
}

fn default_binary_name_for_node_package(package: &str) -> Option<String> {
    if node_package_spec_uses_explicit_source(package) {
        return source_spec_leaf_name(package);
    }
    package_leaf_name(npm_package_name(package))
}

fn default_binary_name_for_cargo_source(source: &CargoInstallSource) -> Option<String> {
    match source {
        CargoInstallSource::RegistryPackage { package, .. } => package_leaf_name(package),
        CargoInstallSource::LocalPath(path) => path_leaf_name(path),
    }
}

fn default_binary_name_for_go_source(source: &GoInstallSource) -> Option<String> {
    match source {
        GoInstallSource::LocalPath(path) => path_leaf_name(path),
        GoInstallSource::PackageSpec(spec) => {
            let package = spec.rsplit_once('@').map(|(path, _)| path).unwrap_or(spec);
            package_leaf_name(package)
        }
    }
}

fn default_binary_name_for_python_package(package: &str) -> Option<String> {
    let trimmed = package.trim();
    let package = trimmed
        .split_once(" @ ")
        .map(|(name, _)| name)
        .unwrap_or(trimmed);
    let package = package
        .split_once(';')
        .map(|(name, _)| name)
        .unwrap_or(package)
        .split_once('[')
        .map(|(name, _)| name)
        .unwrap_or(package);
    let package = package
        .split(['=', '<', '>', '!', '~', ' '])
        .next()
        .unwrap_or(package);
    package_leaf_name(package)
}

fn package_leaf_name(package: &str) -> Option<String> {
    let trimmed = package.trim().trim_end_matches('/');
    let leaf = trimmed.rsplit('/').next().unwrap_or(trimmed).trim();
    if leaf.is_empty() || leaf == "." || leaf == ".." {
        return None;
    }
    Some(leaf.to_string())
}

fn path_leaf_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(ToString::to_string)
}

fn npm_package_name(package: &str) -> &str {
    let package = package.trim();
    if package.starts_with('@') {
        if let Some((name, _version)) = package.rsplit_once('@')
            && name.contains('/')
        {
            return name;
        }
        return package;
    }
    package
        .split_once('@')
        .map(|(name, _)| name)
        .unwrap_or(package)
}

fn source_spec_leaf_name(package: &str) -> Option<String> {
    let package = package.trim();
    if let Some(rest) = package.strip_prefix("npm:") {
        return package_leaf_name(npm_package_name(rest));
    }

    let rest = package
        .strip_prefix("git+")
        .or_else(|| package.split_once(':').map(|(_, rest)| rest))
        .unwrap_or(package);
    let rest = trim_source_spec_suffix(rest);
    package_leaf_name(rest)
}

fn trim_source_spec_suffix(raw: &str) -> &str {
    raw.trim()
        .split(['#', '?'])
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(".git")
        .trim_end_matches('/')
}

fn require_uv_python_version(item_id: &str, raw_version: Option<&str>) -> InstallerResult<String> {
    let version = optional_trimmed(raw_version).ok_or_else(|| {
        InstallerError::usage(format!(
            "plan item `{item_id}` requires non-empty `version`"
        ))
    })?;
    if uv_python_version_selector_is_supported(version) {
        return Ok(version.to_string());
    }
    Err(InstallerError::usage(format!(
        "plan item `{item_id}` with method `uv_python` requires a numeric version selector like `3`, `3.13`, or `3.13.12`"
    )))
}

fn uv_python_version_selector_is_supported(version: &str) -> bool {
    let segments: Vec<_> = version.split('.').collect();
    (1..=3).contains(&segments.len())
        && segments
            .iter()
            .all(|segment| !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit()))
}

fn reject_disallowed_fields(item_id: &str, fields: &[(&str, Option<&str>)]) -> InstallerResult<()> {
    for (name, value) in fields {
        if optional_trimmed(*value).is_some() {
            return Err(InstallerError::usage(format!(
                "plan item `{item_id}` does not allow field `{name}` for this method"
            )));
        }
    }
    Ok(())
}

fn optional_trimmed(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

fn optional_trimmed_owned(raw: Option<&str>) -> Option<String> {
    optional_trimmed(raw).map(ToString::to_string)
}

fn require_non_empty(raw: &str, field_name: &str, item_id: &str) -> InstallerResult<String> {
    optional_trimmed(Some(raw))
        .map(ToString::to_string)
        .ok_or_else(|| {
            InstallerError::usage(format!(
                "plan item `{item_id}` requires non-empty `{field_name}`"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_npm_global_defaults_binary_name_and_manager() {
        let item = InstallPlanItem {
            id: "ruff-cli".to_string(),
            method: "npm_global".to_string(),
            version: Some("0.1.0".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("ruff".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::NpmGlobal(item) = resolved else {
            panic!("expected npm_global plan item");
        };
        assert_eq!(item.binary_name, "ruff");
        assert_eq!(item.package_spec, "ruff@0.1.0");
        assert_eq!(item.manager, NodePackageManager::Npm);
    }

    #[test]
    fn resolve_npm_global_prefers_scoped_package_leaf_over_item_id() {
        let item = InstallPlanItem {
            id: "company-cli".to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("@scope/tool".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::NpmGlobal(item) = resolved else {
            panic!("expected npm_global plan item");
        };
        assert_eq!(item.binary_name, "tool");
    }

    #[test]
    fn resolve_npm_global_appends_version_for_scoped_package() {
        let item = InstallPlanItem {
            id: "scope-tool".to_string(),
            method: "npm_global".to_string(),
            version: Some("1.2.3".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("@scope/pkg".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::NpmGlobal(item) = resolved else {
            panic!("expected npm_global plan item");
        };
        assert_eq!(item.package_spec, "@scope/pkg@1.2.3");
    }

    #[test]
    fn resolve_npm_global_defaults_binary_name_for_npm_source_spec() {
        let item = InstallPlanItem {
            id: "scope-tool".to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("npm:@scope/cli@1.2.3".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::NpmGlobal(item) = resolved else {
            panic!("expected npm_global plan item");
        };
        assert_eq!(item.binary_name, "cli");
    }

    #[test]
    fn resolve_npm_global_defaults_binary_name_for_github_source_spec() {
        let item = InstallPlanItem {
            id: "repo-tool".to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("github:owner/repo#semver:^1".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::NpmGlobal(item) = resolved else {
            panic!("expected npm_global plan item");
        };
        assert_eq!(item.binary_name, "repo");
    }

    #[test]
    fn resolve_cargo_install_treats_plain_package_name_as_registry_package() {
        let item = InstallPlanItem {
            id: "rg-cli".to_string(),
            method: "cargo_install".to_string(),
            version: Some("14.1.0".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("ripgrep".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::CargoInstall(item) = resolved else {
            panic!("expected cargo_install plan item");
        };
        assert_eq!(
            item.source,
            CargoInstallSource::RegistryPackage {
                package: "ripgrep".to_string(),
                version: Some("14.1.0".to_string()),
            }
        );
        assert_eq!(item.binary_name, "ripgrep");
    }

    #[test]
    fn resolve_go_install_treats_plain_package_name_as_remote_package() {
        let item = InstallPlanItem {
            id: "go-cli".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("ripgrep".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::GoInstall(item) = resolved else {
            panic!("expected go_install plan item");
        };
        assert_eq!(
            item.source,
            GoInstallSource::PackageSpec("ripgrep@latest".to_string())
        );
        assert_eq!(item.binary_name, "ripgrep");
    }

    #[test]
    fn resolve_go_install_keeps_module_paths_as_remote_packages() {
        let item = InstallPlanItem {
            id: "formatter".to_string(),
            method: "go_install".to_string(),
            version: Some("v0.7.0".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("mvdan.cc/gofumpt".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::GoInstall(item) = resolved else {
            panic!("expected go_install plan item");
        };
        assert_eq!(
            item.source,
            GoInstallSource::PackageSpec("mvdan.cc/gofumpt@v0.7.0".to_string())
        );
        assert_eq!(item.binary_name, "gofumpt");
    }

    #[test]
    fn resolve_go_install_uses_explicit_local_path_syntax() {
        let item = InstallPlanItem {
            id: "go-local".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./cmd/demo".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::GoInstall(item) = resolved else {
            panic!("expected go_install plan item");
        };
        assert_eq!(
            item.source,
            GoInstallSource::LocalPath(PathBuf::from("cmd/demo"))
        );
        assert_eq!(item.binary_name, "demo");
    }

    #[test]
    fn resolve_go_install_uses_plan_base_dir_for_bare_relative_local_path() {
        let item = InstallPlanItem {
            id: "go-local".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("cmd/demo".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect("resolved");

        let ResolvedPlanItem::GoInstall(item) = resolved else {
            panic!("expected go_install plan item");
        };
        assert_eq!(
            item.source,
            GoInstallSource::LocalPath(PathBuf::from("/repo/plans/cmd/demo"))
        );
        assert_eq!(item.binary_name, "demo");
    }

    #[test]
    fn resolve_go_install_keeps_versioned_bare_path_as_remote_package_spec() {
        let item = InstallPlanItem {
            id: "go-remote".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("cmd/demo@latest".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect("resolved");

        let ResolvedPlanItem::GoInstall(item) = resolved else {
            panic!("expected go_install plan item");
        };
        assert_eq!(
            item.source,
            GoInstallSource::PackageSpec("cmd/demo@latest".to_string())
        );
        assert_eq!(item.binary_name, "demo");
    }

    #[test]
    fn resolve_workspace_package_uses_plan_base_dir_for_relative_destination() {
        let item = InstallPlanItem {
            id: "workspace-demo".to_string(),
            method: "workspace_package".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("./frontend".to_string()),
            package: Some("react".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect("resolved");

        let ResolvedPlanItem::WorkspacePackage(item) = resolved else {
            panic!("expected workspace_package plan item");
        };
        assert_eq!(item.destination, PathBuf::from("/repo/plans/frontend"));
    }

    #[test]
    fn resolve_workspace_package_rejects_version_field() {
        let item = InstallPlanItem {
            id: "workspace-demo".to_string(),
            method: "workspace_package".to_string(),
            version: Some("1.2.3".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("./frontend".to_string()),
            package: Some("react".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect_err("workspace_package should reject version");
        assert!(err.to_string().contains("does not allow field `version`"));
    }

    #[test]
    fn resolve_archive_tree_release_rejects_non_archive_assets() {
        let item = InstallPlanItem {
            id: "tree-demo".to_string(),
            method: "archive_tree_release".to_string(),
            version: None,
            url: Some("https://example.com/demo.bin?download=1".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("archive_tree_release should reject non-archive assets during resolve");
        assert!(
            err.to_string()
                .contains("requires a supported archive asset")
        );
    }

    #[test]
    fn resolve_uv_python_accepts_major_only_selector() {
        let item = InstallPlanItem {
            id: "python3".to_string(),
            method: "uv_python".to_string(),
            version: Some("3".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("major-only selector should be accepted");
        let ResolvedPlanItem::UvPython(item) = resolved else {
            panic!("expected uv_python plan item");
        };
        assert_eq!(item.version, "3");
    }

    #[test]
    fn resolve_uv_tool_defaults_binary_name_from_package() {
        let item = InstallPlanItem {
            id: "lint".to_string(),
            method: "uv_tool".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("ruff".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::UvTool(item) = resolved else {
            panic!("expected uv_tool plan item");
        };
        assert_eq!(item.binary_name, "ruff");
    }

    #[test]
    fn resolve_release_rejects_item_id_with_path_separator() {
        let item = InstallPlanItem {
            id: "../escape".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("path-like id should be rejected");
        assert!(err.to_string().contains("must not contain path separators"));
    }

    #[test]
    fn resolve_release_rejects_binary_name_with_path_separator() {
        let item = InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: Some("../escape".to_string()),
            destination: None,
            package: None,
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("path-like binary_name should be rejected");
        assert!(err.to_string().contains("must not contain path separators"));
    }

    #[test]
    fn resolve_release_normalizes_windows_relative_destination_for_windows_target() {
        let item = InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("bin\\demo.exe".to_string()),
            package: None,
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-pc-windows-msvc",
            "x86_64-pc-windows-msvc",
            None,
        )
        .expect("windows destination");

        let ResolvedPlanItem::Release(item) = resolved else {
            panic!("expected release plan item");
        };
        assert_eq!(
            item.destination,
            Some(PathBuf::from("bin").join("demo.exe"))
        );
    }

    #[test]
    fn resolve_uv_python_rejects_non_numeric_selector() {
        let item = InstallPlanItem {
            id: "python-latest".to_string(),
            method: "uv_python".to_string(),
            version: Some("latest".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("unsupported selector should be rejected");
        assert!(
            err.to_string()
                .contains("requires a numeric version selector")
        );
    }

    #[test]
    fn resolve_cargo_install_uses_plan_base_dir_for_local_path() {
        let item = InstallPlanItem {
            id: "cargo-demo".to_string(),
            method: "cargo_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./tools/demo".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect("resolved");

        let ResolvedPlanItem::CargoInstall(item) = resolved else {
            panic!("expected cargo_install plan item");
        };
        assert_eq!(
            item.source,
            CargoInstallSource::LocalPath(PathBuf::from("/repo/plans/tools/demo"))
        );
    }

    #[test]
    fn resolve_system_package_parses_explicit_manager() {
        let item = InstallPlanItem {
            id: "git".to_string(),
            method: "system_package".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("git".to_string()),
            manager: Some("dnf".to_string()),
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect("resolved");

        let ResolvedPlanItem::SystemPackage(item) = resolved else {
            panic!("expected system_package plan item");
        };
        assert_eq!(
            item.mode,
            SystemPackageMode::Explicit(SystemPackageManager::Dnf)
        );
    }
}
