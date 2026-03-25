use crate::error::{OperationError, OperationResult};

use super::uv_installation_source_candidates::InstallationSourceCandidate;

pub(super) fn attempt_source_candidates<T, F>(
    candidates: Vec<InstallationSourceCandidate>,
    failure_context: &str,
    mut execute_candidate: F,
) -> OperationResult<T>
where
    F: FnMut(InstallationSourceCandidate) -> Result<T, String>,
{
    let mut errors = Vec::new();
    for candidate in candidates {
        match execute_candidate(candidate) {
            Ok(output) => return Ok(output),
            Err(err) => errors.push(err),
        }
    }

    Err(OperationError::install(format!(
        "{failure_context}: {}",
        errors.join(" | ")
    )))
}
