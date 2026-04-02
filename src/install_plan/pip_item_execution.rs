use std::ffi::OsString;

use omne_process_primitives::{HostRecipeRequest, command_exists, run_host_recipe};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::PipPlanItem;

pub(crate) fn execute_pip_item(item: &PipPlanItem) -> OperationResult<BootstrapItem> {
    execute_pip_item_with(item, command_exists, |python, args| {
        run_host_recipe(&HostRecipeRequest::new(python, args)).map_err(|err| err.to_string())
    })
}

fn execute_pip_item_with<CommandExists, RunRecipe>(
    item: &PipPlanItem,
    command_exists_fn: CommandExists,
    run_recipe: RunRecipe,
) -> OperationResult<BootstrapItem>
where
    CommandExists: Fn(&str) -> bool,
    RunRecipe: Fn(&str, &[OsString]) -> Result<(), String>,
{
    let candidates = pip_python_candidates(item);
    let args = pip_install_args(item);

    let mut probe_failures = Vec::new();
    for python in candidates {
        if !command_exists_fn(&python) {
            probe_failures.push(format!("{python} not found"));
            continue;
        }
        match run_recipe(python.as_ref(), &args) {
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
            Err(err) => {
                let prefix = if probe_failures.is_empty() {
                    String::new()
                } else {
                    format!("{} | ", probe_failures.join(" | "))
                };
                return Err(OperationError::install(format!(
                    "{prefix}{python} failed: {err}"
                )));
            }
        }
    }
    Err(OperationError::install(format!(
        "all pip recipes failed: {}",
        probe_failures.join(" | ")
    )))
}

fn pip_python_candidates(item: &PipPlanItem) -> Vec<String> {
    if let Some(explicit_python) = item.python.as_ref() {
        return vec![explicit_python.clone()];
    }
    vec!["python3".to_string(), "python".to_string()]
}

fn pip_install_args(item: &PipPlanItem) -> Vec<OsString> {
    vec![
        "-m".to_string(),
        "pip".to_string(),
        "install".to_string(),
        item.package.clone(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{execute_pip_item_with, pip_install_args, pip_python_candidates};
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

    #[test]
    fn default_python_fallback_only_runs_when_python3_is_missing() {
        let item = PipPlanItem {
            id: "pip-demo".to_string(),
            package: "ruff".to_string(),
            python: None,
        };
        let attempts = RefCell::new(Vec::new());

        let result = execute_pip_item_with(
            &item,
            |command| command == "python",
            |python, args| {
                attempts
                    .borrow_mut()
                    .push((python.to_string(), args.to_vec()));
                Ok(())
            },
        )
        .expect("python fallback should run when python3 is missing");

        assert_eq!(result.source.as_deref(), Some("pip:python"));
        assert_eq!(
            attempts.into_inner(),
            vec![("python".to_string(), pip_install_args(&item))]
        );
    }

    #[test]
    fn default_python_fallback_stops_after_python3_install_failure() {
        let item = PipPlanItem {
            id: "pip-demo".to_string(),
            package: "ruff".to_string(),
            python: None,
        };
        let attempts = RefCell::new(Vec::new());

        let err = execute_pip_item_with(
            &item,
            |_command| true,
            |python, _args| {
                attempts.borrow_mut().push(python.to_string());
                Err("boom".to_string())
            },
        )
        .expect_err("python3 failure should stop fallback");

        assert_eq!(attempts.into_inner(), vec!["python3".to_string()]);
        assert!(err.to_string().contains("python3 failed: boom"));
    }
}
