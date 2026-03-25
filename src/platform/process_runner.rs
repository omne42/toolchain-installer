use std::ffi::OsStr;
use std::path::Path;

use omne_process_primitives::{
    HostCommandRequest, HostCommandSudoMode, command_available as runtime_command_available,
    command_exists as runtime_command_exists, command_path_exists as runtime_command_path_exists,
    run_host_command,
};

use crate::error::{OperationError, OperationResult};

pub(crate) fn run_recipe(program: &str, args: &[String]) -> OperationResult<()> {
    run_recipe_with_env(OsStr::new(program), args, &[])
}

pub(crate) fn run_recipe_with_env(
    program: &OsStr,
    args: &[String],
    env: &[(String, String)],
) -> OperationResult<()> {
    let sudo_mode = if program
        .to_str()
        .is_some_and(|value| value.eq_ignore_ascii_case("brew"))
    {
        // Homebrew explicitly rejects root execution, so macOS package installs must stay direct.
        HostCommandSudoMode::Never
    } else {
        HostCommandSudoMode::IfNonRootSystemCommand
    };
    let output = run_host_command(&HostCommandRequest {
        program,
        args,
        env,
        sudo_mode,
    })
    .map_err(|err| OperationError::install(err.to_string()))?
    .output;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!(
        "status={} stderr={} stdout={}",
        output.status, stderr, stdout
    );
    Err(OperationError::install(combined))
}

pub(crate) fn command_exists(command: &str) -> bool {
    runtime_command_exists(command)
}

pub(crate) fn command_path_exists(command: &Path) -> bool {
    runtime_command_path_exists(command)
}

pub(crate) fn command_available(command: &str) -> bool {
    runtime_command_available(command)
}
