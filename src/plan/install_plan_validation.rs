use std::path::Path;

use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};
use omne_integrity_primitives::parse_sha256_user_input;
use omne_system_package_primitives::SystemPackageManager;

use crate::contracts::{InstallPlan, InstallPlanItem, PLAN_SCHEMA_VERSION};
use crate::error::{InstallerError, InstallerResult};
use crate::plan::plan_method::{ManagedToolchainMethod, PlanMethod, normalize_plan_method};
pub fn validate_install_plan(
    plan: &InstallPlan,
    requested_target_triple: Option<&str>,
) -> InstallerResult<()> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(requested_target_triple, &host_triple);
    validate_plan(plan, &host_triple, &target_triple)
}

pub(crate) fn validate_plan(
    plan: &InstallPlan,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<()> {
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

    for item in &plan.items {
        validate_plan_item(item, host_triple, target_triple)?;
    }

    Ok(())
}

fn validate_plan_item(
    item: &InstallPlanItem,
    host_triple: &str,
    target_triple: &str,
) -> InstallerResult<()> {
    let id = item.id.trim();
    if id.is_empty() {
        return Err(InstallerError::usage("plan item `id` cannot be empty"));
    }

    let Some(normalized_method) = normalize_plan_method(&item.method) else {
        return Err(InstallerError::usage(format!(
            "plan item `{}` has an empty `method`",
            item.id
        )));
    };
    let method = PlanMethod::from_normalized(&normalized_method);

    if target_triple != host_triple && method.is_host_bound() {
        return Err(InstallerError::usage(format!(
            "plan item `{}` uses host-bound method `{normalized_method}` but target triple `{target_triple}` does not match host triple `{host_triple}`",
            item.id
        )));
    }

    match method {
        PlanMethod::Release => {
            reject_unset_fields(
                &item.id,
                &[
                    ("version", item.version.as_deref()),
                    ("package", item.package.as_deref()),
                    ("manager", item.manager.as_deref()),
                    ("python", item.python.as_deref()),
                ],
            )?;
            validate_release_url(&item.id, item.url.as_deref())?;
            validate_destination_input(&item.id, item.destination.as_deref())?;
            if item
                .url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` with method `release` requires `url`",
                    item.id
                )));
            }
            if let Some(raw_sha256) = item.sha256.as_deref()
                && parse_sha256_user_input(raw_sha256).is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` has invalid `sha256` value",
                    item.id
                )));
            }
        }
        PlanMethod::Apt | PlanMethod::SystemPackage => {
            reject_unset_fields(
                &item.id,
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
            if item
                .package
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` with method `{normalized_method}` requires `package`",
                    item.id
                )));
            }
            if let Some(manager) = item
                .manager
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                && SystemPackageManager::parse(manager).is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` uses unsupported manager `{manager}`",
                    item.id
                )));
            }
            if matches!(method, PlanMethod::Apt)
                && let Some(manager) = item.manager.as_deref().map(str::trim)
                && !manager.is_empty()
                && !matches!(
                    SystemPackageManager::parse(manager),
                    Some(SystemPackageManager::AptGet)
                )
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` uses method `apt` but manager `{manager}`",
                    item.id
                )));
            }
        }
        PlanMethod::Pip => {
            reject_unset_fields(
                &item.id,
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
            if item
                .package
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` with method `pip` requires `package`",
                    item.id
                )));
            }
        }
        PlanMethod::ManagedToolchain(managed_method) => {
            validate_managed_toolchain_plan_item(managed_method, item)?
        }
        PlanMethod::Unknown => {}
    }

    Ok(())
}

fn validate_managed_toolchain_plan_item(
    method: ManagedToolchainMethod,
    item: &InstallPlanItem,
) -> InstallerResult<()> {
    match method {
        ManagedToolchainMethod::Uv => {
            reject_unset_fields(
                &item.id,
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
        }
        ManagedToolchainMethod::UvPython => {
            reject_unset_fields(
                &item.id,
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
            if item
                .version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` with method `uv_python` requires `version`",
                    item.id
                )));
            }
        }
        ManagedToolchainMethod::UvTool => {
            reject_unset_fields(
                &item.id,
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
            if item
                .package
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(InstallerError::usage(format!(
                    "plan item `{}` with method `uv_tool` requires `package`",
                    item.id
                )));
            }
        }
    }

    Ok(())
}

fn reject_unset_fields(item_id: &str, fields: &[(&str, Option<&str>)]) -> InstallerResult<()> {
    for (name, value) in fields {
        if value.map(str::trim).is_some_and(|raw| !raw.is_empty()) {
            return Err(InstallerError::usage(format!(
                "plan item `{item_id}` does not allow field `{name}` for this method"
            )));
        }
    }
    Ok(())
}

fn validate_release_url(item_id: &str, raw_url: Option<&str>) -> InstallerResult<()> {
    let Some(url) = raw_url.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let parsed = reqwest::Url::parse(url).map_err(|err| {
        InstallerError::usage(format!("plan item `{item_id}` has invalid `url`: {err}"))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(InstallerError::usage(format!(
            "plan item `{item_id}` uses unsupported url scheme `{other}`"
        ))),
    }
}

fn validate_destination_input(item_id: &str, destination: Option<&str>) -> InstallerResult<()> {
    let Some(raw_destination) = destination.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let path = Path::new(raw_destination);
    if path.file_name().is_none() {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` destination `{raw_destination}` must include a file name"
        )));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(InstallerError::usage(format!(
            "plan item `{item_id}` destination `{raw_destination}` cannot contain `..`"
        )));
    }
    Ok(())
}
