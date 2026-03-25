use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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
    run_recipe_with_env_in_dir(program, args, env, None)
}

pub(crate) fn run_recipe_with_env_in_dir(
    program: &OsStr,
    args: &[String],
    env: &[(String, String)],
    working_directory: Option<&Path>,
) -> OperationResult<()> {
    let sudo_mode = sudo_mode_for_program(program);
    let output = run_host_command(&HostCommandRequest {
        program,
        args,
        env,
        working_directory,
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

fn sudo_mode_for_program(program: &OsStr) -> HostCommandSudoMode {
    let Some(program) = program.to_str() else {
        return HostCommandSudoMode::Never;
    };
    match program {
        // Homebrew explicitly rejects root execution, so macOS package installs must stay direct.
        "brew" => HostCommandSudoMode::Never,
        "apt-get" | "dnf" | "yum" | "apk" | "pacman" | "zypper" => {
            HostCommandSudoMode::IfNonRootSystemCommand
        }
        _ => HostCommandSudoMode::Never,
    }
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

pub(crate) fn resolve_command_for_execution(command: &str) -> String {
    resolve_command_path(command)
        .and_then(|path| path.into_os_string().into_string().ok())
        .unwrap_or_else(|| command.to_string())
}

pub(crate) fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    #[cfg(windows)]
    let pathexts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect();

    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let has_ext = Path::new(command).extension().is_some();
            if has_ext {
                continue;
            }
            for ext in &pathexts {
                let candidate = dir.join(format!("{command}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use omne_process_primitives::HostCommandSudoMode;

    use super::sudo_mode_for_program;

    #[test]
    fn system_package_managers_may_use_auto_sudo() {
        assert_eq!(
            sudo_mode_for_program(OsStr::new("apt-get")),
            HostCommandSudoMode::IfNonRootSystemCommand
        );
        assert_eq!(
            sudo_mode_for_program(OsStr::new("dnf")),
            HostCommandSudoMode::IfNonRootSystemCommand
        );
    }

    #[test]
    fn user_space_install_commands_stay_direct() {
        assert_eq!(
            sudo_mode_for_program(OsStr::new("cargo")),
            HostCommandSudoMode::Never
        );
        assert_eq!(
            sudo_mode_for_program(OsStr::new("go")),
            HostCommandSudoMode::Never
        );
        assert_eq!(
            sudo_mode_for_program(OsStr::new("npm")),
            HostCommandSudoMode::Never
        );
        assert_eq!(
            sudo_mode_for_program(OsStr::new("rustup")),
            HostCommandSudoMode::Never
        );
    }
}
