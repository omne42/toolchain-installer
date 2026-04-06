use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const VERSION_PROBE_ATTEMPTS: usize = 3;
const VERSION_PROBE_RETRY_DELAY: Duration = Duration::from_millis(100);

pub(crate) fn binary_reports_version(path: &Path) -> bool {
    run_version_probe_with_retries(path).is_some_and(|probe| probe.success)
}

pub(crate) fn binary_reports_version_with_prefix(path: &Path, expected_prefix: &str) -> bool {
    run_version_probe_with_retries(path).is_some_and(|probe| {
        probe.success
            && String::from_utf8_lossy(&probe.stdout)
                .lines()
                .chain(String::from_utf8_lossy(&probe.stderr).lines())
                .any(|line| line.starts_with(expected_prefix))
    })
}

pub(crate) fn python_binary_version(path: &Path) -> Option<String> {
    run_version_probe_with_retries(path).and_then(|probe| {
        probe
            .success
            .then(|| {
                python_version_from_output(&probe.stdout)
                    .or_else(|| python_version_from_output(&probe.stderr))
            })
            .flatten()
    })
}

#[cfg(test)]
pub(crate) fn python_binary_matches_version(path: &Path, expected_version: &str) -> bool {
    python_binary_version(path).is_some_and(|reported_version| {
        python_version_matches_requirement(&reported_version, expected_version)
    })
}

struct VersionProbeOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_version_probe_with_retries(path: &Path) -> Option<VersionProbeOutput> {
    let mut last_probe = None;

    for attempt in 0..VERSION_PROBE_ATTEMPTS {
        let probe = run_version_probe(path);
        let should_retry = match probe.as_ref() {
            Some(output) => !output.success,
            None => true,
        };
        last_probe = probe;

        if !should_retry || attempt + 1 == VERSION_PROBE_ATTEMPTS {
            return last_probe;
        }

        thread::sleep(VERSION_PROBE_RETRY_DELAY);
    }

    last_probe
}

fn run_version_probe(path: &Path) -> Option<VersionProbeOutput> {
    run_version_probe_with_timeout(path, VERSION_PROBE_TIMEOUT)
}

fn run_version_probe_with_timeout(path: &Path, timeout: Duration) -> Option<VersionProbeOutput> {
    if !path.exists() {
        return None;
    }

    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take().map(spawn_probe_reader);
    let stderr = child.stderr.take().map(spawn_probe_reader);
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Some(finish_version_probe(status.success(), stdout, stderr));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(VERSION_PROBE_POLL_INTERVAL),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn spawn_probe_reader<R>(mut reader: R) -> JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    })
}

fn finish_version_probe(
    success: bool,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
) -> VersionProbeOutput {
    VersionProbeOutput {
        success,
        stdout: join_probe_reader(stdout),
        stderr: join_probe_reader(stderr),
    }
}

fn join_probe_reader(reader: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn python_version_from_output(output: &[u8]) -> Option<String> {
    let output = String::from_utf8_lossy(output);
    output.lines().find_map(|line| {
        let mut segments = line.split_whitespace();
        match (segments.next(), segments.next(), segments.next()) {
            (Some("Python"), Some(version), None) => Some(version.to_string()),
            _ => None,
        }
    })
}

#[cfg(test)]
fn python_version_matches_requirement(reported_version: &str, expected_version: &str) -> bool {
    reported_version == expected_version
        || reported_version.starts_with(&format!("{expected_version}."))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::VERSION_PROBE_TIMEOUT;
    use super::python_version_matches_requirement;
    use super::{
        binary_reports_version, python_binary_matches_version, run_version_probe_with_timeout,
    };

    fn portable_unix_script(body: &str) -> String {
        format!("#!/usr/bin/env bash\n{body}")
    }

    fn large_output_script_body(
        stdout_len: usize,
        stderr_len: usize,
        stderr_prefix: &str,
    ) -> String {
        format!(
            r#"emit_repeat() {{
  local size="$1"
  local char="$2"
  if [ "$size" -le 0 ]; then
    return 0
  fi
  yes "$char" | tr -d '\n' | head -c "$size"
}}
if [ "$1" != "--version" ]; then
  exit 2
fi
{stderr_prefix}
emit_repeat {stdout_len} x
printf '\n'
emit_repeat {stderr_len} y >&2
printf '\n' >&2
"#
        )
    }

    #[test]
    fn python_version_match_uses_component_boundaries() {
        assert!(python_version_matches_requirement("3.13.2", "3"));
        assert!(python_version_matches_requirement("3.13.2", "3.13"));
        assert!(python_version_matches_requirement("3.13.2", "3.13.2"));
        assert!(!python_version_matches_requirement("3.10.8", "3.1"));
        assert!(!python_version_matches_requirement("3.13.2", "3.13.20"));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn binary_reports_version_retries_after_transient_probe_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state_path = tmp.path().join("probe-state");
        let script_path = tmp.path().join("uv");

        write_unix_probe_script(
            &script_path,
            &portable_unix_script(&format!(
                r#"if [ "$1" != "--version" ]; then
  exit 2
fi
if [ ! -f "{}" ]; then
  touch "{}"
  exit 42
fi
echo "uv 0.11.0"
"#,
                state_path.display(),
                state_path.display()
            )),
        );

        assert!(binary_reports_version(&script_path));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn python_binary_matches_version_retries_after_transient_probe_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state_path = tmp.path().join("probe-state");
        let script_path = tmp.path().join("python3.13");

        write_unix_probe_script(
            &script_path,
            &portable_unix_script(&format!(
                r#"if [ "$1" != "--version" ]; then
  exit 2
fi
if [ ! -f "{}" ]; then
  touch "{}"
  exit 42
fi
echo "Python 3.13.12"
"#,
                state_path.display(),
                state_path.display()
            )),
        );

        assert!(python_binary_matches_version(&script_path, "3.13.12"));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn version_probe_handles_large_pipe_output_without_deadlock() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        let output_len = 65_536;

        write_unix_probe_script(
            &script_path,
            &portable_unix_script(&large_output_script_body(
                output_len,
                0,
                "printf 'uv 0.11.0\\n' >&2",
            )),
        );

        // CI runners, especially macOS, can be heavily oversubscribed. Keep this comfortably
        // above the default timeout and only write enough data to overrun typical pipe buffers.
        let probe = run_version_probe_with_timeout(&script_path, VERSION_PROBE_TIMEOUT * 30)
            .expect("probe output");
        assert!(probe.success);
        assert!(
            String::from_utf8_lossy(&probe.stderr)
                .lines()
                .any(|line| line == "uv 0.11.0")
        );
        assert!(probe.stdout.len() >= output_len);
    }

    fn write_unix_probe_script(path: &Path, body: &str) {
        fs::write(path, body).expect("write probe script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path).expect("stat probe script").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod probe script");
        }
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn version_probe_drains_large_stdout_and_stderr_without_deadlock() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        let output_len = 32_768;

        write_unix_probe_script(
            &script_path,
            &portable_unix_script(&large_output_script_body(output_len, output_len, "")),
        );

        // Full `cargo test --all-targets` can contend heavily with compile jobs on CI and
        // shared runners. Keep this probe comfortably above the default timeout so we only fail
        // on an actual pipe-drain regression, not on transient scheduler starvation.
        let probe = run_version_probe_with_timeout(&script_path, VERSION_PROBE_TIMEOUT * 30)
            .expect("probe output");

        assert!(probe.success);
        assert!(probe.stdout.len() >= output_len);
        assert!(probe.stderr.len() >= output_len);
    }
}
