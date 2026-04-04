use std::ffi::{OsStr, OsString};
use std::process::Output;
use std::time::Duration;

use omne_process_primitives::{
    HostCommandError, HostCommandRunOptions, HostCommandSudoMode, HostRecipeError,
    HostRecipeRequest, run_host_recipe_with_options,
};

const MANAGED_UV_RECIPE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(crate) fn run_managed_uv_recipe(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
) -> Result<Output, String> {
    run_managed_uv_recipe_with_timeout(program, args, env, MANAGED_UV_RECIPE_TIMEOUT)
}

fn run_managed_uv_recipe_with_timeout(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<Output, String> {
    let removed_env = inherited_uv_environment_names();
    let request = HostRecipeRequest::new(program, args)
        .with_env(env)
        .with_sudo_mode(HostCommandSudoMode::Never);
    let options = HostCommandRunOptions::new()
        .with_env_remove(&removed_env)
        .with_timeout(timeout);

    match run_host_recipe_with_options(&request, options) {
        Ok(output) => Ok(output.output),
        Err(HostRecipeError::NonZeroExit {
            program, output, ..
        }) => Err(render_redacted_managed_uv_failure(
            format!("run {} failed", program.to_string_lossy()),
            output.status,
            &output.stderr,
            &output.stdout,
        )),
        Err(HostRecipeError::Command(err)) => Err(render_managed_uv_command_error(err)),
    }
}

fn render_managed_uv_command_error(err: HostCommandError) -> String {
    match err {
        HostCommandError::TimedOut {
            program,
            timeout,
            output,
            ..
        } => render_redacted_managed_uv_failure(
            format!(
                "run {} timed out after {}s",
                program.to_string_lossy(),
                timeout.as_secs()
            ),
            output.status,
            &output.stderr,
            &output.stdout,
        ),
        other => other.to_string(),
    }
}

fn render_redacted_managed_uv_failure(
    prefix: String,
    status: std::process::ExitStatus,
    stderr: &[u8],
    stdout: &[u8],
) -> String {
    format!(
        "{prefix}: status={status} stderr_bytes={} stdout_bytes={} (captured output redacted)",
        stderr.len(),
        stdout.len(),
    )
}

fn inherited_uv_environment_names() -> Vec<OsString> {
    std::env::vars_os()
        .filter_map(|(name, _)| {
            name.to_str()
                .is_some_and(|value| value.to_ascii_uppercase().starts_with("UV_"))
                .then_some(name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use super::run_managed_uv_recipe_with_timeout;

    fn portable_unix_script(body: &str) -> String {
        format!("#!/usr/bin/env bash\n{body}")
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_times_out_hung_process() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"sleep 5
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_timeout(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_millis(50),
        )
        .expect_err("hung process should time out");

        assert!(err.contains("timed out"));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_timeout_reports_captured_stdio() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"echo "stdout-before-timeout"
echo "stderr-before-timeout" >&2
sleep 5
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_timeout(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_millis(50),
        )
        .expect_err("hung process should time out");

        assert!(err.contains("stderr_bytes=22"), "{err}");
        assert!(err.contains("stdout_bytes=22"), "{err}");
        assert!(err.contains("captured output redacted"), "{err}");
        assert!(!err.contains("stdout-before-timeout"), "{err}");
        assert!(!err.contains("stderr-before-timeout"), "{err}");
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_redacts_non_zero_exit_stdio() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"printf '0123456789abcdefghijklmnopqrstuvwxyz'
exit 42
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_timeout(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_secs(1),
        )
        .expect_err("non-zero exit should fail");

        assert!(err.contains("stdout_bytes=36"), "{err}");
        assert!(err.contains("stderr_bytes=0"), "{err}");
        assert!(err.contains("captured output redacted"), "{err}");
        assert!(
            !err.contains("0123456789abcdefghijklmnopqrstuvwxyz"),
            "{err}"
        );
    }

    fn write_unix_script(path: &Path, body: &str) {
        fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path).expect("stat script").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod script");
        }
    }
}
