use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MANAGED_UV_RECIPE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MANAGED_UV_RECIPE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MANAGED_UV_RECIPE_OUTPUT_LIMIT: usize = 64 * 1024;

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
    run_managed_uv_recipe_with_limits(program, args, env, timeout, MANAGED_UV_RECIPE_OUTPUT_LIMIT)
}

fn run_managed_uv_recipe_with_limits(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
    timeout: Duration,
    output_limit: usize,
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
    let stdout = spawn_stream_capture(child.stdout.take(), output_limit);
    let stderr = spawn_stream_capture(child.stderr.take(), output_limit);
    let deadline = Instant::now() + timeout;

    let (status, stdout, stderr) = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = finish_stream_capture(stdout)?;
                let stderr = finish_stream_capture(stderr)?;
                break (status, stdout, stderr);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(MANAGED_UV_RECIPE_POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let status = child
                    .wait()
                    .map_err(|err| format!("run {program} timed out and wait failed: {err}"))?;
                let stdout = finish_stream_capture(stdout)?;
                let stderr = finish_stream_capture(stderr)?;
                return Err(format!(
                    "run {program} timed out after {}s: status={} stderr={} stdout={}",
                    timeout.as_secs(),
                    status,
                    render_captured_stream(&stderr),
                    render_captured_stream(&stdout),
                ));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = finish_stream_capture(stdout);
                let _ = finish_stream_capture(stderr);
                return Err(format!("run {program} failed while waiting: {err}"));
            }
        }
    };
    if status.success() {
        return Ok(Output {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
        });
    }

    Err(format!(
        "run {program} failed: status={} stderr={} stdout={}",
        status,
        render_captured_stream(&stderr),
        render_captured_stream(&stdout),
    ))
}

fn spawn_stream_capture(
    handle: Option<impl Read + Send + 'static>,
    output_limit: usize,
) -> Option<JoinHandle<CapturedStream>> {
    handle.map(|mut handle| {
        thread::spawn(move || {
            let mut capture = CapturedStream::new(output_limit);
            let mut chunk = [0u8; 8192];
            loop {
                match handle.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => capture.push(&chunk[..read]),
                    Err(_) => break,
                }
            }
            capture
        })
    })
}

fn finish_stream_capture(
    handle: Option<JoinHandle<CapturedStream>>,
) -> Result<CapturedStream, String> {
    match handle {
        Some(handle) => handle
            .join()
            .map_err(|_| "managed uv output capture thread panicked".to_string()),
        None => Ok(CapturedStream::new(0)),
    }
}

fn render_captured_stream(stream: &CapturedStream) -> String {
    let mut rendered = String::from_utf8_lossy(&stream.bytes).into_owned();
    if stream.truncated {
        rendered.push_str(&format!(" [truncated after {} bytes]", stream.output_limit));
    }
    rendered
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

struct CapturedStream {
    bytes: Vec<u8>,
    output_limit: usize,
    truncated: bool,
}

impl CapturedStream {
    fn new(output_limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            output_limit,
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let retained = self.bytes.len();
        if retained < self.output_limit {
            let remaining = self.output_limit - retained;
            let to_copy = remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..to_copy]);
        }
        if bytes.len() > self.output_limit.saturating_sub(retained) {
            self.truncated = true;
        }
        if self.bytes.len() > self.output_limit {
            self.bytes.truncate(self.output_limit);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use super::{run_managed_uv_recipe_with_limits, run_managed_uv_recipe_with_timeout};

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

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_limits_captured_output() {
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

        let err = run_managed_uv_recipe_with_limits(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_secs(1),
            16,
        )
        .expect_err("large output should be truncated");

        assert!(err.contains("0123456789abcdef"));
        assert!(err.contains("[truncated after 16 bytes]"));
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
