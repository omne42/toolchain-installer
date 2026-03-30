use std::ffi::OsString;

use omne_process_primitives::{HostRecipeRequest, command_exists, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::PipPlanItem;

pub(crate) fn execute_pip_item(item: &PipPlanItem) -> OperationResult<BootstrapItem> {
    let preferred_python = item.python.clone().unwrap_or_else(|| "python3".to_string());
    let candidates = if preferred_python == "python3" {
        vec!["python3".to_string(), "python".to_string()]
    } else {
        vec![preferred_python]
    };

    let mut errors = Vec::new();
    for python in candidates {
        if !command_exists(&python) {
            errors.push(format!("{python} not found"));
            continue;
        }
        let args = vec![
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            item.package.clone(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        match run_host_recipe(&HostRecipeRequest::new(python.as_ref(), &args)) {
            Ok(_) => {
                return Ok(BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Installed,
                    source: Some(format!("pip:{python}")),
                    source_kind: Some(BootstrapSourceKind::Pip),
                    archive_match: None,
                    destination: None,
                    detail: None,
                    error_code: None,
                    failure_code: None,
                });
            }
            Err(err) => errors.push(format!("{python} failed: {err}")),
        }
    }
    Err(OperationError::install(format!(
        "all pip recipes failed: {}",
        errors.join(" | ")
    )))
}
