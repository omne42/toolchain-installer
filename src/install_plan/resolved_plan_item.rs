use std::path::PathBuf;

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

use super::item_destination_resolution::validate_destination;
use super::plan_method::{
    ManagedToolchainMethod, PlanMethod, SUPPORTED_PLAN_METHODS, normalize_plan_method,
};

pub(crate) fn resolve_plan_item(
    item: &InstallPlanItem,
    host_triple: &str,
    target_triple: &str,
    plan_base_dir: Option<&std::path::Path>,
) -> InstallerResult<ResolvedPlanItem> {
    let id = require_non_empty(item.id.as_str(), "id", "plan item")?;

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
        PlanMethod::Release => resolve_release_plan_item(item, id),
        PlanMethod::ArchiveTreeRelease => resolve_archive_tree_release_plan_item(item, id),
        PlanMethod::SystemPackage => resolve_system_package_plan_item(item, id),
        PlanMethod::Apt => resolve_apt_plan_item(item, id),
        PlanMethod::Pip => resolve_pip_plan_item(item, id),
        PlanMethod::NpmGlobal => resolve_npm_global_plan_item(item, id),
        PlanMethod::WorkspacePackage => resolve_workspace_package_plan_item(item, id),
        PlanMethod::CargoInstall => resolve_cargo_install_plan_item(item, id, plan_base_dir),
        PlanMethod::RustupComponent => resolve_rustup_component_plan_item(item, id),
        PlanMethod::GoInstall => resolve_go_install_plan_item(item, id, plan_base_dir),
        PlanMethod::ManagedToolchain(method) => {
            resolve_managed_toolchain_plan_item(item, id, method)
        }
        PlanMethod::Unknown => unreachable!("unsupported method should fail before resolve"),
    }
}

fn resolve_release_plan_item(
    item: &InstallPlanItem,
    id: String,
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
    let destination = parse_optional_destination(&id, item.destination.as_deref())?;
    Ok(ResolvedPlanItem::Release(ReleasePlanItem {
        id,
        url,
        sha256,
        archive_binary: optional_trimmed_owned(item.archive_binary.as_deref()),
        binary_name: optional_trimmed_owned(item.binary_name.as_deref()),
        destination,
    }))
}

fn resolve_archive_tree_release_plan_item(
    item: &InstallPlanItem,
    id: String,
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
    let sha256 = parse_optional_sha256(&id, item.sha256.as_deref())?;
    let destination = parse_optional_destination(&id, item.destination.as_deref())?;
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
    Ok(ResolvedPlanItem::NpmGlobal(NpmGlobalPlanItem {
        package_spec: build_versioned_package_spec(&package, version),
        manager: parse_node_package_manager(&id, item.manager.as_deref(), "npm_global")?,
        binary_name: optional_trimmed_owned(item.binary_name.as_deref())
            .unwrap_or_else(|| id.clone()),
        id,
    }))
}

fn resolve_workspace_package_plan_item(
    item: &InstallPlanItem,
    id: String,
) -> InstallerResult<ResolvedPlanItem> {
    reject_disallowed_fields(
        &id,
        &[
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
    let version = optional_trimmed(item.version.as_deref());
    Ok(ResolvedPlanItem::WorkspacePackage(
        WorkspacePackagePlanItem {
            package_spec: build_versioned_package_spec(&package, version),
            manager: parse_node_package_manager(&id, item.manager.as_deref(), "workspace_package")?,
            destination: require_destination(&id, item.destination.as_deref())?,
            id,
        },
    ))
}

fn resolve_cargo_install_plan_item(
    item: &InstallPlanItem,
    id: String,
    plan_base_dir: Option<&std::path::Path>,
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
    Ok(ResolvedPlanItem::CargoInstall(CargoInstallPlanItem {
        binary_name: optional_trimmed_owned(item.binary_name.as_deref())
            .unwrap_or_else(|| id.clone()),
        id,
        source: resolve_cargo_install_source(&package, version, plan_base_dir),
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
    Ok(ResolvedPlanItem::RustupComponent(RustupComponentPlanItem {
        id,
        component: require_non_empty(
            item.package.as_deref().unwrap_or_default(),
            "package",
            item.id.as_str(),
        )?,
        binary_name: optional_trimmed_owned(item.binary_name.as_deref()),
    }))
}

fn resolve_go_install_plan_item(
    item: &InstallPlanItem,
    id: String,
    plan_base_dir: Option<&std::path::Path>,
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
        GoInstallSource::LocalPath(resolve_local_plan_path(&package, plan_base_dir))
    } else if package.contains('@') {
        GoInstallSource::PackageSpec(package)
    } else {
        let version = optional_trimmed(item.version.as_deref()).unwrap_or("latest");
        GoInstallSource::PackageSpec(format!("{package}@{version}"))
    };
    Ok(ResolvedPlanItem::GoInstall(GoInstallPlanItem {
        binary_name: optional_trimmed_owned(item.binary_name.as_deref())
            .unwrap_or_else(|| id.clone()),
        id,
        source,
    }))
}

fn resolve_managed_toolchain_plan_item(
    item: &InstallPlanItem,
    id: String,
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
                version: require_non_empty(
                    item.version.as_deref().unwrap_or_default(),
                    "version",
                    item.id.as_str(),
                )?,
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
            Ok(ResolvedPlanItem::UvTool(UvToolPlanItem {
                package: require_non_empty(
                    item.package.as_deref().unwrap_or_default(),
                    "package",
                    item.id.as_str(),
                )?,
                python: optional_trimmed_owned(item.python.as_deref()),
                binary_name: optional_trimmed_owned(item.binary_name.as_deref())
                    .unwrap_or_else(|| id.clone()),
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
    plan_base_dir: Option<&std::path::Path>,
) -> CargoInstallSource {
    if looks_like_explicit_cargo_local_path(package) {
        return CargoInstallSource::LocalPath(resolve_local_plan_path(package, plan_base_dir));
    }
    CargoInstallSource::RegistryPackage {
        package: package.to_string(),
        version: version.map(ToString::to_string),
    }
}

fn resolve_local_plan_path(package: &str, plan_base_dir: Option<&std::path::Path>) -> PathBuf {
    let candidate = PathBuf::from(package);
    if candidate.is_absolute()
        || package.starts_with('\\')
        || looks_like_windows_drive_path(package)
    {
        return candidate;
    }
    match plan_base_dir {
        Some(base_dir) => base_dir.join(candidate),
        None => candidate,
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
        || looks_like_windows_drive_path(package)
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
) -> InstallerResult<Option<PathBuf>> {
    optional_trimmed(raw_destination)
        .map(|destination| validate_destination(item_id, destination))
        .transpose()
}

fn require_destination(item_id: &str, raw_destination: Option<&str>) -> InstallerResult<PathBuf> {
    parse_optional_destination(item_id, raw_destination)?.ok_or_else(|| {
        InstallerError::usage(format!(
            "plan item `{item_id}` with method `workspace_package` requires `destination`"
        ))
    })
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
            id: "ruff".to_string(),
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
    fn resolve_cargo_install_treats_plain_package_name_as_registry_package() {
        let item = InstallPlanItem {
            id: "ripgrep".to_string(),
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
    }

    #[test]
    fn resolve_go_install_treats_plain_package_name_as_remote_package() {
        let item = InstallPlanItem {
            id: "ripgrep".to_string(),
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
    }

    #[test]
    fn resolve_go_install_keeps_module_paths_as_remote_packages() {
        let item = InstallPlanItem {
            id: "gofumpt".to_string(),
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
    }

    #[test]
    fn resolve_go_install_uses_explicit_local_path_syntax() {
        let item = InstallPlanItem {
            id: "demo".to_string(),
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
            GoInstallSource::LocalPath(PathBuf::from("./cmd/demo"))
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
