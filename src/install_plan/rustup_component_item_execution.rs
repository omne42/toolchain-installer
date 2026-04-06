use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use omne_process_primitives::{HostRecipeRequest, resolve_command_path};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::host_recipe::run_installer_host_recipe;
use crate::installer_runtime_config::DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS;
use crate::plan_items::RustupComponentPlanItem;

#[allow(dead_code)]
pub(crate) fn execute_rustup_component_item(
    item: &RustupComponentPlanItem,
    _target_triple: &str,
    _managed_dir: &std::path::Path,
) -> OperationResult<BootstrapItem> {
    execute_rustup_component_item_with_timeout(
        item,
        _target_triple,
        _managed_dir,
        Duration::from_secs(DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS),
    )
}

pub(crate) fn execute_rustup_component_item_with_timeout(
    item: &RustupComponentPlanItem,
    _target_triple: &str,
    _managed_dir: &std::path::Path,
    timeout: Duration,
) -> OperationResult<BootstrapItem> {
    let args = vec![
        "component".to_string(),
        "add".to_string(),
        item.component.to_string(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    run_installer_host_recipe(&HostRecipeRequest::new("rustup".as_ref(), &args), timeout)?;

    let destination =
        resolve_rustup_component_destination(item).map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("rustup:component:{}", item.component)),
        source_kind: Some(BootstrapSourceKind::RustupComponent),
        archive_match: None,
        destination: destination.map(|path| path.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

fn resolve_rustup_component_destination(
    item: &RustupComponentPlanItem,
) -> Result<Option<PathBuf>, String> {
    resolve_rustup_component_destination_with(item, resolve_command_path)
}

fn resolve_rustup_component_destination_with<F>(
    item: &RustupComponentPlanItem,
    resolve_binary: F,
) -> Result<Option<PathBuf>, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    if let Some(binary_name) = item.binary_name.as_deref() {
        return resolve_binary(binary_name).map(Some).ok_or_else(|| {
            format!(
                "rustup_component `{}` succeeded but explicit binary_name `{}` was not found in PATH",
                item.component, binary_name
            )
        });
    }

    Ok(find_rustup_component_binary(&item.component)
        .as_deref()
        .and_then(resolve_binary))
}

fn find_rustup_component_binary(component: &str) -> Option<String> {
    let binary_name = match component {
        "rustfmt" => "rustfmt",
        "clippy" => "cargo-clippy",
        _ => return None,
    };
    Some(binary_name.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::resolve_rustup_component_destination_with;
    use crate::plan_items::RustupComponentPlanItem;

    #[test]
    fn explicit_binary_name_is_authoritative_for_destination_resolution() {
        let item = RustupComponentPlanItem {
            id: "rustfmt".to_string(),
            component: "rustfmt".to_string(),
            binary_name: Some("custom-rustfmt".to_string()),
        };

        let err =
            resolve_rustup_component_destination_with(&item, |binary_name| match binary_name {
                "rustfmt" => Some(PathBuf::from("/fake/rustfmt")),
                _ => None,
            })
            .expect_err("explicit binary_name should not silently fall back");

        assert!(err.contains("custom-rustfmt"));
    }

    #[test]
    fn implicit_binary_name_falls_back_to_known_component_binary() {
        let item = RustupComponentPlanItem {
            id: "rustfmt".to_string(),
            component: "rustfmt".to_string(),
            binary_name: None,
        };

        let destination =
            resolve_rustup_component_destination_with(&item, |binary_name| match binary_name {
                "rustfmt" => Some(PathBuf::from("/fake/rustfmt")),
                _ => None,
            })
            .expect("known component binary should resolve");

        assert_eq!(destination, Some(PathBuf::from("/fake/rustfmt")));
    }
}
