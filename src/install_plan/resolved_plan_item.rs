use std::path::{Path, PathBuf};

use omne_artifact_install_primitives::is_archive_tree_asset_name;
use omne_integrity_primitives::{Sha256Digest, parse_sha256_user_input};
use omne_system_package_primitives::SystemPackageManager;
use reqwest::Url;

use crate::contracts::InstallPlanItem;
use crate::error::{InstallerError, InstallerResult};
use crate::plan_items::{
    ArchiveTreeReleasePlanItem, CargoInstallPlanItem, CargoInstallSource, GoInstallPlanItem,
    GoInstallSource, HostPackageInput, ManagedUvPlanItem, NodePackageManager, NpmGlobalPlanItem,
    PipPlanItem, ReleasePlanItem, ResolvedPlanItem, RustupComponentPlanItem, SystemPackageMode,
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
        PlanMethod::Pip => resolve_pip_plan_item(item, id, host_triple, plan_base_dir),
        PlanMethod::NpmGlobal => resolve_npm_global_plan_item(item, id, host_triple, plan_base_dir),
        PlanMethod::WorkspacePackage => {
            resolve_workspace_package_plan_item(item, id, host_triple, target_triple, plan_base_dir)
        }
        PlanMethod::CargoInstall => {
            resolve_cargo_install_plan_item(item, id, host_triple, target_triple, plan_base_dir)
        }
        PlanMethod::RustupComponent => resolve_rustup_component_plan_item(item, id),
        PlanMethod::GoInstall => {
            resolve_go_install_plan_item(item, id, host_triple, target_triple, plan_base_dir)
        }
        PlanMethod::ManagedToolchain(method) => {
            resolve_managed_toolchain_plan_item(item, id, target_triple, plan_base_dir, method)
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

    let apt_alias = item.method.trim().eq_ignore_ascii_case("apt");
    let mode = if apt_alias {
        match optional_trimmed(item.manager.as_deref()) {
            None | Some("apt-get") => SystemPackageMode::Explicit(SystemPackageManager::AptGet),
            Some(manager) => {
                return Err(InstallerError::usage(format!(
                    "plan item `{id}` with method `apt` only accepts `manager=apt-get`, got `{manager}`"
                )));
            }
        }
    } else if let Some(manager) = optional_trimmed(item.manager.as_deref()) {
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

fn resolve_pip_plan_item(
    item: &InstallPlanItem,
    id: String,
    host_triple: &str,
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
            ("destination", item.destination.as_deref()),
            ("manager", item.manager.as_deref()),
        ],
    )?;
    Ok(ResolvedPlanItem::Pip(PipPlanItem {
        id,
        package: resolve_host_package_input(
            item.package.as_deref().unwrap_or_default(),
            "pip",
            item.id.as_str(),
            Some(host_triple),
            plan_base_dir,
        )?,
        python: optional_trimmed_owned(item.python.as_deref()),
    }))
}

fn resolve_npm_global_plan_item(
    item: &InstallPlanItem,
    id: String,
    host_triple: &str,
    plan_base_dir: Option<&Path>,
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
    let package = resolve_host_package_input(
        item.package.as_deref().unwrap_or_default(),
        "npm_global",
        item.id.as_str(),
        Some(host_triple),
        plan_base_dir,
    )?;
    let version = optional_trimmed(item.version.as_deref());
    reject_conflicting_npm_global_version(&id, &package, version)?;
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
    let package = require_host_package_input(
        item.package.as_deref().unwrap_or_default(),
        "workspace_package",
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
    host_triple: &str,
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
    let package = require_host_package_input(
        item.package.as_deref().unwrap_or_default(),
        "cargo_install",
        item.id.as_str(),
    )?;
    let version = optional_trimmed(item.version.as_deref());
    reject_conflicting_cargo_install_version(&id, &package, version)?;
    let source = resolve_cargo_install_source(&package, version, plan_base_dir, host_triple, &id)?;
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
    let component = require_host_package_input(
        item.package.as_deref().unwrap_or_default(),
        "rustup_component",
        item.id.as_str(),
    )?;
    let binary_name = validate_rustup_component_binary_name(
        &id,
        &component,
        parse_optional_binary_name(&id, item.binary_name.as_deref())?,
    )?;
    Ok(ResolvedPlanItem::RustupComponent(RustupComponentPlanItem {
        id,
        component,
        binary_name,
    }))
}

fn resolve_go_install_plan_item(
    item: &InstallPlanItem,
    id: String,
    host_triple: &str,
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
    let package = require_host_package_input(
        item.package.as_deref().unwrap_or_default(),
        "go_install",
        item.id.as_str(),
    )?;
    reject_conflicting_go_install_version(
        &id,
        &package,
        optional_trimmed(item.version.as_deref()),
    )?;
    let source = if looks_like_explicit_go_local_path(&package) {
        reject_non_native_windows_local_path(&package, host_triple, &id, "go_install")?;
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
    plan_base_dir: Option<&Path>,
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
            let package = resolve_host_package_input(
                item.package.as_deref().unwrap_or_default(),
                "uv_tool",
                item.id.as_str(),
                Some(target_triple),
                plan_base_dir,
            )?;
            Ok(ResolvedPlanItem::UvTool(UvToolPlanItem {
                package: package.clone(),
                python: optional_trimmed_owned(item.python.as_deref()),
                binary_name: parse_optional_binary_name(&id, item.binary_name.as_deref())?
                    .or_else(|| default_binary_name_for_python_package(&package))
                    .unwrap_or_else(|| default_binary_name_for_target(target_triple, &id)),
                binary_name_explicit,
                id,
            }))
        }
    }
}

fn reject_conflicting_npm_global_version(
    item_id: &str,
    package: &HostPackageInput,
    version: Option<&str>,
) -> InstallerResult<()> {
    let Some(version) = version else {
        return Ok(());
    };
    let package_display = package.render();
    let Some(package_spec) = package.as_package_spec() else {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `npm_global` cannot set `version` to `{version}` because `package` already encodes an explicit source or local path: `{package_display}`"
        )));
    };
    if node_package_spec_has_embedded_version(package_spec) {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `npm_global` cannot set `version` to `{version}` because `package` already encodes a version: `{package_display}`"
        )));
    }
    if node_package_spec_uses_explicit_source(package_spec) {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `npm_global` cannot set `version` to `{version}` because `package` already encodes an explicit source or local path: `{package_display}`"
        )));
    }
    Ok(())
}

fn reject_conflicting_go_install_version(
    item_id: &str,
    package: &str,
    version: Option<&str>,
) -> InstallerResult<()> {
    let Some(version) = version else {
        return Ok(());
    };
    if looks_like_explicit_go_local_path(package) {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `go_install` cannot set `version` to `{version}` because local `package` paths must encode their own source: `{package}`"
        )));
    }
    if package.contains('@') {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `go_install` cannot set `version` to `{version}` because `package` already encodes a version: `{package}`"
        )));
    }
    Ok(())
}

fn reject_conflicting_cargo_install_version(
    item_id: &str,
    package: &str,
    version: Option<&str>,
) -> InstallerResult<()> {
    let Some(version) = version else {
        return Ok(());
    };
    if looks_like_explicit_cargo_local_path(package) {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `cargo_install` cannot set `version` to `{version}` because local `package` paths must encode their own source: `{package}`"
        )));
    }
    Ok(())
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

fn build_versioned_package_spec(
    package: &HostPackageInput,
    version: Option<&str>,
) -> HostPackageInput {
    let Some(package_spec) = package.as_package_spec() else {
        return package.clone();
    };
    if let Some(version) = version {
        if node_package_spec_has_embedded_version(package_spec)
            || node_package_spec_uses_explicit_source(package_spec)
        {
            return package.clone();
        }
        return HostPackageInput::package_spec(format!("{package_spec}@{version}"));
    }
    package.clone()
}

fn resolve_cargo_install_source(
    package: &str,
    version: Option<&str>,
    plan_base_dir: Option<&Path>,
    host_triple: &str,
    item_id: &str,
) -> InstallerResult<CargoInstallSource> {
    if looks_like_explicit_cargo_local_path(package) {
        reject_non_native_windows_local_path(package, host_triple, item_id, "cargo_install")?;
        return Ok(CargoInstallSource::LocalPath(resolve_plan_relative_path(
            Path::new(package),
            plan_base_dir,
        )));
    }
    Ok(CargoInstallSource::RegistryPackage {
        package: package.to_string(),
        version: version.map(ToString::to_string),
    })
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
    if npm_alias_source_target(package).is_some() {
        return true;
    }
    if node_package_spec_is_local_path(package) {
        return true;
    }
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
        || package.contains("@npm:")
}

fn node_package_spec_is_local_path(package: &str) -> bool {
    looks_like_explicit_host_local_path(package)
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

fn default_binary_name_for_node_package(package: &HostPackageInput) -> Option<String> {
    if let Some(path) = package.as_local_path() {
        return path_leaf_name(path);
    }
    let package = package.as_package_spec()?;
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

fn default_binary_name_for_rustup_component(component: &str) -> Option<&'static str> {
    match component {
        "rustfmt" => Some("rustfmt"),
        "clippy" => Some("cargo-clippy"),
        _ => None,
    }
}

fn default_binary_name_for_python_package(package: &HostPackageInput) -> Option<String> {
    let Some(package) = package.as_package_spec() else {
        return package.as_local_path().and_then(path_leaf_name);
    };
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
    let leaf = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
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

fn validate_rustup_component_binary_name(
    item_id: &str,
    component: &str,
    binary_name: Option<String>,
) -> InstallerResult<Option<String>> {
    let Some(binary_name) = binary_name else {
        return Ok(None);
    };

    let Some(expected_binary) = default_binary_name_for_rustup_component(component) else {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `rustup_component` does not support `binary_name` for component `{component}` because installer cannot verify a stable CLI entrypoint"
        )));
    };

    if binary_name != expected_binary {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `rustup_component` requires `binary_name={expected_binary}` for component `{component}`, got `{binary_name}`"
        )));
    }

    Ok(Some(binary_name))
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

fn resolve_host_package_input(
    raw_package: &str,
    method: &str,
    item_id: &str,
    host_triple: Option<&str>,
    plan_base_dir: Option<&Path>,
) -> InstallerResult<HostPackageInput> {
    let package = require_host_package_input(raw_package, method, item_id)?;
    Ok(
        match resolve_host_package_local_path(
            &package,
            plan_base_dir,
            host_triple,
            item_id,
            method,
        )? {
            Some(path) => HostPackageInput::LocalPath(path),
            None => HostPackageInput::PackageSpec(package),
        },
    )
}

fn require_host_package_input(
    raw_package: &str,
    method: &str,
    item_id: &str,
) -> InstallerResult<String> {
    let package = require_non_empty(raw_package, "package", item_id)?;
    reject_option_like_host_package(item_id, method, &package)?;
    Ok(package)
}

fn reject_option_like_host_package(
    item_id: &str,
    method: &str,
    package: &str,
) -> InstallerResult<()> {
    if package.starts_with('-') {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `{method}` does not allow `package` to look like a command-line option"
        )));
    }
    Ok(())
}

fn resolve_host_package_local_path(
    package: &str,
    plan_base_dir: Option<&Path>,
    host_triple: Option<&str>,
    item_id: &str,
    method: &str,
) -> InstallerResult<Option<PathBuf>> {
    if let Some(local_path) = package
        .strip_prefix("file:")
        .filter(|value| !value.trim().is_empty() && !value.starts_with("//"))
    {
        if let Some(host_triple) = host_triple {
            reject_non_native_windows_local_path(local_path, host_triple, item_id, method)?;
        }
        let resolved = resolve_plan_relative_path(Path::new(local_path), plan_base_dir);
        return Ok(Some(resolved));
    }
    if looks_like_explicit_host_local_path(package) {
        if let Some(host_triple) = host_triple {
            reject_non_native_windows_local_path(package, host_triple, item_id, method)?;
        }
        let resolved = resolve_plan_relative_path(Path::new(package), plan_base_dir);
        return Ok(Some(resolved));
    }
    Ok(None)
}

fn looks_like_explicit_host_local_path(package: &str) -> bool {
    let package = package.trim();
    if npm_alias_source_target(package).is_some() {
        return false;
    }
    if package.contains("://")
        || package.starts_with("git+")
        || package.starts_with("git:")
        || package.starts_with("github:")
        || package.starts_with("workspace:")
        || package.starts_with("link:")
        || package.starts_with("npm:")
        || package.contains("@npm:")
    {
        return false;
    }
    package == "."
        || package == ".."
        || package.starts_with("./")
        || package.starts_with(".\\")
        || package.starts_with("../")
        || package.starts_with("..\\")
        || package.starts_with('/')
        || package.starts_with('\\')
        || looks_like_windows_drive_path(package)
        || (package.contains(['/', '\\']) && !package.starts_with('@'))
}

fn reject_non_native_windows_local_path(
    raw_path: &str,
    host_triple: &str,
    item_id: &str,
    method: &str,
) -> InstallerResult<()> {
    if host_triple.contains("windows") {
        return Ok(());
    }
    if looks_like_windows_drive_path(raw_path) || raw_path.starts_with('\\') {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` with method `{method}` uses Windows-local path syntax `{raw_path}` but host triple `{host_triple}` does not use Windows path semantics"
        )));
    }
    Ok(())
}

fn source_spec_leaf_name(package: &str) -> Option<String> {
    let package = package.trim();
    if let Some(target) = npm_alias_source_target(package) {
        return package_leaf_name(npm_package_name(target));
    }
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

fn npm_alias_source_target(package: &str) -> Option<&str> {
    package
        .trim()
        .split_once("@npm:")
        .map(|(_, target)| target.trim())
        .filter(|target| !target.is_empty())
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
        assert_eq!(
            item.package_spec,
            HostPackageInput::package_spec("ruff@0.1.0")
        );
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
        assert_eq!(
            item.package_spec,
            HostPackageInput::package_spec("@scope/pkg@1.2.3")
        );
    }

    #[test]
    fn resolve_npm_global_rejects_version_when_package_already_has_version() {
        let item = InstallPlanItem {
            id: "scope-tool".to_string(),
            method: "npm_global".to_string(),
            version: Some("9.9.9".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("@scope/pkg@1.2.3".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("conflicting versions should fail");

        assert!(err.to_string().contains("already encodes a version"));
    }

    #[test]
    fn resolve_npm_global_rejects_version_when_package_uses_explicit_source() {
        let item = InstallPlanItem {
            id: "scope-tool".to_string(),
            method: "npm_global".to_string(),
            version: Some("9.9.9".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("file:../packages/cli".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("explicit sources should reject separate version");

        assert!(
            err.to_string()
                .contains("already encodes an explicit source or local path")
        );
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
    fn resolve_go_install_rejects_version_when_package_already_has_version() {
        let item = InstallPlanItem {
            id: "demo".to_string(),
            method: "go_install".to_string(),
            version: Some("latest".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("example.com/demo/cmd/demo@v1.2.3".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("conflicting go versions should fail");

        assert!(err.to_string().contains("already encodes a version"));
    }

    #[test]
    fn resolve_go_install_rejects_version_for_local_package_path() {
        let item = InstallPlanItem {
            id: "demo".to_string(),
            method: "go_install".to_string(),
            version: Some("latest".to_string()),
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./cmd/demo".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("local go package path should reject separate version");

        assert!(err.to_string().contains("local `package` paths"));
    }

    #[test]
    fn resolve_npm_global_defaults_binary_name_for_npm_alias_source_spec() {
        let item = InstallPlanItem {
            id: "scope-tool".to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("company-cli@npm:@scope/cli@1.2.3".to_string()),
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
        assert_eq!(
            item.package_spec,
            HostPackageInput::package_spec("company-cli@npm:@scope/cli@1.2.3")
        );
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
    fn resolve_pip_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "pip-demo".to_string(),
            method: "pip".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("--editable".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("option-like pip package should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_pip_uses_plan_base_dir_for_relative_local_path() {
        let plan_base = Path::new("/repo/plans");
        let item = InstallPlanItem {
            id: "pip-demo".to_string(),
            method: "pip".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./packages/demo".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(plan_base),
        )
        .expect("resolved");

        let ResolvedPlanItem::Pip(item) = resolved else {
            panic!("expected pip plan item");
        };
        assert_eq!(
            item.package.as_local_path(),
            Some(plan_base.join("packages").join("demo").as_path())
        );
    }

    #[test]
    fn resolve_npm_global_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "npm-demo".to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("--workspace".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("option-like npm_global package should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_workspace_package_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "workspace-demo".to_string(),
            method: "workspace_package".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("apps/demo".to_string()),
            package: Some("--workspace".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect_err("option-like workspace package should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_npm_global_uses_plan_base_dir_for_relative_local_path() {
        let plan_base = Path::new("/repo/plans");
        let item = InstallPlanItem {
            id: "npm-demo".to_string(),
            method: "npm_global".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./packages/demo".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(plan_base),
        )
        .expect("resolved");

        let ResolvedPlanItem::NpmGlobal(item) = resolved else {
            panic!("expected npm_global plan item");
        };
        assert_eq!(
            item.package_spec.as_local_path(),
            Some(plan_base.join("packages").join("demo").as_path())
        );
        assert_eq!(item.binary_name, "demo");
    }

    #[test]
    fn resolve_uv_tool_uses_plan_base_dir_for_relative_local_path() {
        let plan_base = Path::new("/repo/plans");
        let item = InstallPlanItem {
            id: "uv-tool-demo".to_string(),
            method: "uv_tool".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./packages/demo".to_string()),
            manager: None,
            python: None,
        };

        let resolved = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(plan_base),
        )
        .expect("resolved");

        let ResolvedPlanItem::UvTool(item) = resolved else {
            panic!("expected uv_tool plan item");
        };
        assert_eq!(
            item.package.as_local_path(),
            Some(plan_base.join("packages").join("demo").as_path())
        );
        assert_eq!(item.binary_name, "demo");
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
    fn resolve_cargo_install_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "cargo-demo".to_string(),
            method: "cargo_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("--git".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("option-like cargo package should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_cargo_install_rejects_windows_absolute_local_path_on_non_windows_host() {
        let item = InstallPlanItem {
            id: "cargo-demo".to_string(),
            method: "cargo_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("C:\\repo\\demo".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect_err("non-windows host should reject windows-local cargo path");
        assert!(err.to_string().contains("Windows-local path syntax"));
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
    fn resolve_go_install_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "go-demo".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("--mod".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("option-like go package should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_go_install_rejects_windows_root_relative_local_path_on_non_windows_host() {
        let item = InstallPlanItem {
            id: "go-demo".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("\\repo\\demo".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect_err("non-windows host should reject windows-local go path");
        assert!(err.to_string().contains("Windows-local path syntax"));
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
    fn resolve_uv_tool_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "lint".to_string(),
            method: "uv_tool".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("--index-url".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("option-like uv_tool package should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_pip_rejects_windows_absolute_file_source_on_non_windows_host() {
        let item = InstallPlanItem {
            id: "pip-demo".to_string(),
            method: "pip".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("file:C:\\repo\\demo".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(Path::new("/repo/plans")),
        )
        .expect_err("non-windows host should reject windows-local pip file path");
        assert!(err.to_string().contains("Windows-local path syntax"));
    }

    #[test]
    fn resolve_rustup_component_rejects_option_like_package_input() {
        let item = InstallPlanItem {
            id: "rustfmt-demo".to_string(),
            method: "rustup_component".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("--toolchain".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("option-like rustup component should be rejected");
        assert!(err.to_string().contains("look like a command-line option"));
    }

    #[test]
    fn resolve_rustup_component_rejects_unknown_component_binary_name_override() {
        let item = InstallPlanItem {
            id: "rust-src-demo".to_string(),
            method: "rustup_component".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: Some("rust-src".to_string()),
            destination: None,
            package: Some("rust-src".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("unknown rustup component binary should be rejected");
        assert!(
            err.to_string()
                .contains("does not support `binary_name` for component `rust-src`")
        );
    }

    #[test]
    fn resolve_rustup_component_rejects_mismatched_binary_name_override() {
        let item = InstallPlanItem {
            id: "rustfmt-demo".to_string(),
            method: "rustup_component".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: Some("cargo-rustfmt".to_string()),
            destination: None,
            package: Some("rustfmt".to_string()),
            manager: None,
            python: None,
        };

        let err = resolve_plan_item(
            &item,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        )
        .expect_err("mismatched rustup component binary should be rejected");
        assert!(
            err.to_string()
                .contains("requires `binary_name=rustfmt` for component `rustfmt`")
        );
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
