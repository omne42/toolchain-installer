use std::ffi::OsString;
use std::path::Path;

use omne_process_primitives::{HostRecipeRequest, resolve_command_path, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::WorkspacePackagePlanItem;

pub(crate) fn execute_workspace_package_item(
    item: &WorkspacePackagePlanItem,
    _managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    let workspace_dir = item.destination.clone();
    if !workspace_dir.join("package.json").exists() {
        return Err(OperationError::install(format!(
            "workspace_package requires an existing package.json under {}",
            workspace_dir.display()
        )));
    }
    let (program, args) = build_workspace_package_command(item)?;
    let manager = item.manager.command_name();
    run_host_recipe(&HostRecipeRequest::new(program.as_os_str(), &args))
        .map_err(OperationError::from_host_recipe)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("workspace:{manager}")),
        source_kind: Some(BootstrapSourceKind::WorkspacePackage),
        archive_match: None,
        destination: Some(workspace_dir.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

fn build_workspace_package_command(
    item: &WorkspacePackagePlanItem,
) -> OperationResult<(OsString, Vec<OsString>)> {
    let workspace_dir = &item.destination;
    let args = match item.manager.command_name() {
        "npm" => vec![
            OsString::from("install"),
            OsString::from("--prefix"),
            workspace_dir.as_os_str().to_os_string(),
            OsString::from(&item.package_spec),
        ],
        "pnpm" => vec![
            OsString::from("add"),
            OsString::from("--dir"),
            workspace_dir.as_os_str().to_os_string(),
            OsString::from(&item.package_spec),
        ],
        "bun" => vec![
            OsString::from("add"),
            OsString::from("--cwd"),
            workspace_dir.as_os_str().to_os_string(),
            OsString::from(&item.package_spec),
        ],
        value => {
            return Err(OperationError::install(format!(
                "unsupported workspace_package manager `{value}`"
            )));
        }
    };
    let manager = item.manager.command_name();
    let program = resolve_command_path(manager)
        .map(|path| path.into_os_string())
        .unwrap_or_else(|| OsString::from(manager));
    Ok((program, args))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::build_workspace_package_command;
    use crate::plan_items::{NodePackageManager, WorkspacePackagePlanItem};

    #[cfg(unix)]
    #[test]
    fn workspace_package_command_preserves_non_utf8_destination_arg() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let destination = PathBuf::from(OsString::from_vec(b"/tmp/workspace-\xff".to_vec()));
        let item = WorkspacePackagePlanItem {
            id: "workspace-demo".to_string(),
            package_spec: "eslint".to_string(),
            manager: NodePackageManager::Npm,
            destination,
        };

        let (_program, args) =
            build_workspace_package_command(&item).expect("build workspace command");

        assert_eq!(args[2].as_bytes(), b"/tmp/workspace-\xff");
    }
}
