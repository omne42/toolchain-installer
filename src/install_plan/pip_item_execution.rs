use std::ffi::OsString;

use omne_process_primitives::{HostRecipeRequest, command_exists, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::PipPlanItem;

pub(crate) fn execute_pip_item(item: &PipPlanItem) -> OperationResult<BootstrapItem> {
    let candidates = pip_python_candidates(item);

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

fn pip_python_candidates(item: &PipPlanItem) -> Vec<String> {
    if let Some(explicit_python) = item.python.as_ref() {
        return vec![explicit_python.clone()];
    }
    vec!["python3".to_string(), "python".to_string()]
}

#[cfg(test)]
mod tests {
    use super::pip_python_candidates;
    use crate::plan_items::PipPlanItem;

    #[test]
    fn explicit_python3_does_not_fall_back_to_python() {
        let item = PipPlanItem {
            id: "pip-demo".to_string(),
            package: "ruff".to_string(),
            python: Some("python3".to_string()),
        };

        assert_eq!(pip_python_candidates(&item), vec!["python3".to_string()]);
    }

    #[test]
    fn default_python_candidates_keep_python3_then_python_fallback() {
        let item = PipPlanItem {
            id: "pip-demo".to_string(),
            package: "ruff".to_string(),
            python: None,
        };

        assert_eq!(
            pip_python_candidates(&item),
            vec!["python3".to_string(), "python".to_string()]
        );
    }
}
