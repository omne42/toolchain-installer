use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::{command_exists, run_recipe};

pub(crate) fn execute_pip_item(item: &InstallPlanItem) -> OperationResult<BootstrapItem> {
    let package = item
        .package
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("pip method requires `package`"))?;
    let preferred_python = item
        .python
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".to_string());
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
            package.clone(),
        ];
        match run_recipe(&python, &args) {
            Ok(_) => {
                return Ok(BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Installed,
                    source: Some(format!("pip:{python}")),
                    source_kind: Some(BootstrapSourceKind::Pip),
                    archive_match: None,
                    destination: item.destination.clone(),
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
