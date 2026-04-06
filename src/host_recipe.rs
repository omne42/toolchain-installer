use std::time::Duration;

use omne_process_primitives::{
    HostCommandError, HostCommandOutput, HostCommandRunOptions, HostRecipeError, HostRecipeRequest,
    run_host_recipe_with_options,
};

use crate::error::{OperationError, OperationResult};

const HOST_RECIPE_OUTPUT_LIMIT: usize = 64 * 1024;

pub(crate) fn run_installer_host_recipe(
    request: &HostRecipeRequest<'_>,
    timeout: Duration,
) -> OperationResult<HostCommandOutput> {
    let options = HostCommandRunOptions::new().with_timeout(timeout);
    match run_host_recipe_with_options(request, options) {
        Ok(output) => Ok(output),
        Err(err) => Err(map_host_recipe_error(err)),
    }
}

fn map_host_recipe_error(err: HostRecipeError) -> OperationError {
    match err {
        HostRecipeError::Command(HostCommandError::TimedOut {
            program,
            timeout,
            output,
            ..
        }) => OperationError::install(format!(
            "run {} timed out after {}s: status={} stderr={} stdout={}",
            program.to_string_lossy(),
            timeout.as_secs(),
            output.status,
            render_captured_bytes(&output.stderr),
            render_captured_bytes(&output.stdout),
        )),
        other => OperationError::from_host_recipe(other),
    }
}

fn render_captured_bytes(bytes: &[u8]) -> String {
    let retained = bytes.len().min(HOST_RECIPE_OUTPUT_LIMIT);
    let mut rendered = String::from_utf8_lossy(&bytes[..retained]).into_owned();
    if bytes.len() > HOST_RECIPE_OUTPUT_LIMIT {
        rendered.push_str(&format!(
            " [truncated after {} bytes]",
            HOST_RECIPE_OUTPUT_LIMIT
        ));
    }
    rendered
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use omne_process_primitives::HostRecipeRequest;

    use super::run_installer_host_recipe;

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

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn installer_host_recipe_reports_timeout_with_captured_stdio() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script_path = temp.path().join("hang");
        write_unix_script(
            &script_path,
            "#!/usr/bin/env bash\necho stdout-before-timeout\necho stderr-before-timeout >&2\nsleep 5\n",
        );

        let args: Vec<OsString> = Vec::new();
        let err = run_installer_host_recipe(
            &HostRecipeRequest::new(script_path.as_os_str(), &args),
            Duration::from_millis(50),
        )
        .expect_err("hung process should time out");

        assert!(err.to_string().contains("timed out"));
        assert!(err.to_string().contains("stdout-before-timeout"));
        assert!(err.to_string().contains("stderr-before-timeout"));
    }

    #[cfg_attr(windows, ignore = "shell probe is unix-specific")]
    #[test]
    fn installer_host_recipe_preserves_nonzero_exit_errors() {
        let args = vec![OsString::from("-c"), OsString::from("exit 7")];
        let err = run_installer_host_recipe(
            &HostRecipeRequest::new("sh".as_ref(), &args),
            Duration::from_secs(1),
        )
        .expect_err("non-zero exit should remain an error");

        assert!(err.to_string().contains("status="));
    }
}
