use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MANAGED_UV_RECIPE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MANAGED_UV_RECIPE_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    let mut command = Command::new(program);
    command.args(args);
    for name in inherited_uv_environment_names() {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .spawn()
        .map_err(|err| format!("run {program} failed: {err}"))?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let deadline = Instant::now() + timeout;

    let output = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = read_child_stream(stdout.take());
                let stderr = read_child_stream(stderr.take());
                break Output {
                    status,
                    stdout,
                    stderr,
                };
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(MANAGED_UV_RECIPE_POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let status = child
                    .wait()
                    .map_err(|err| format!("run {program} timed out and wait failed: {err}"))?;
                let stdout = read_child_stream(stdout.take());
                let stderr = read_child_stream(stderr.take());
                return Err(format!(
                    "run {program} timed out after {}s: status={} stderr={} stdout={}",
                    timeout.as_secs(),
                    status,
                    String::from_utf8_lossy(&stderr),
                    String::from_utf8_lossy(&stdout),
                ));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("run {program} failed while waiting: {err}"));
            }
        }
    };
    if output.status.success() {
        return Ok(output);
    }

    Err(format!(
        "run {program} failed: status={} stderr={} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    ))
}

fn read_child_stream(handle: Option<impl Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut handle) = handle {
        let _ = handle.read_to_end(&mut bytes);
    }
    bytes
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

        assert!(err.contains("stdout-before-timeout"));
        assert!(err.contains("stderr-before-timeout"));
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
