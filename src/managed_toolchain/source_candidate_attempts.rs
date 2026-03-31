use crate::error::{ExitCode, OperationError, OperationResult};

use super::uv_installation_source_candidates::InstallationSourceCandidate;

pub(super) fn attempt_source_candidates<T, F>(
    candidates: Vec<InstallationSourceCandidate>,
    failure_context: &str,
    mut execute_candidate: F,
) -> OperationResult<T>
where
    F: FnMut(InstallationSourceCandidate) -> OperationResult<T>,
{
    let mut errors = Vec::new();
    let mut saw_install_failure = false;
    for candidate in candidates {
        match execute_candidate(candidate) {
            Ok(output) => return Ok(output),
            Err(err) => {
                saw_install_failure |= err.exit_code() != ExitCode::Download;
                errors.push(err.detail());
            }
        }
    }

    let detail = if errors.is_empty() {
        failure_context.to_string()
    } else {
        format!("{failure_context}: {}", errors.join(" | "))
    };
    if saw_install_failure {
        return Err(OperationError::install(detail));
    }
    Err(OperationError::download(detail))
}

#[cfg(test)]
mod tests {
    use crate::contracts::BootstrapSourceKind;

    use super::*;

    fn candidate(label: &str) -> InstallationSourceCandidate {
        InstallationSourceCandidate {
            label: label.to_string(),
            env: Vec::new(),
            probe_url: None,
            source_kind: BootstrapSourceKind::Canonical,
        }
    }

    #[test]
    fn attempt_source_candidates_keeps_download_failure_when_all_candidates_download_fail() {
        let result: OperationResult<()> = attempt_source_candidates(
            vec![candidate("official"), candidate("mirror")],
            "all uv_python sources failed",
            |candidate| {
                Err(OperationError::download(format!(
                    "{} failed",
                    candidate.label
                )))
            },
        );
        let err = result.expect_err("all-download failure should stay download-scoped");

        assert_eq!(err.exit_code(), ExitCode::Download);
        assert!(err.detail().contains("official failed"));
        assert!(err.detail().contains("mirror failed"));
    }

    #[test]
    fn attempt_source_candidates_promotes_mixed_failures_to_install_failure() {
        let result: OperationResult<()> = attempt_source_candidates(
            vec![candidate("official"), candidate("mirror")],
            "all uv_tool sources failed",
            |candidate| {
                if candidate.label == "official" {
                    return Err(OperationError::download("official failed"));
                }
                Err(OperationError::install("mirror produced a broken binary"))
            },
        );
        let err = result.expect_err("install-side candidate failures should remain install-scoped");

        assert_eq!(err.exit_code(), ExitCode::Install);
        assert!(err.detail().contains("official failed"));
        assert!(err.detail().contains("mirror produced a broken binary"));
    }
}
