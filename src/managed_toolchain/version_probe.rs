use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn binary_reports_version(path: &Path) -> bool {
    run_version_probe(path).is_some_and(|probe| probe.success)
}

pub(crate) fn python_binary_matches_version(path: &Path, expected_version: &str) -> bool {
    run_version_probe(path).is_some_and(|probe| {
        probe.success && python_version_output_matches(&probe.stdout, expected_version)
            || probe.success && python_version_output_matches(&probe.stderr, expected_version)
    })
}

struct VersionProbeOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_version_probe(path: &Path) -> Option<VersionProbeOutput> {
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
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout_bytes = Vec::new();
                if let Some(mut handle) = stdout.take() {
                    let _ = handle.read_to_end(&mut stdout_bytes);
                }

                let mut stderr_bytes = Vec::new();
                if let Some(mut handle) = stderr.take() {
                    let _ = handle.read_to_end(&mut stderr_bytes);
                }

                return Some(VersionProbeOutput {
                    success: status.success(),
                    stdout: stdout_bytes,
                    stderr: stderr_bytes,
                });
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

fn python_version_output_matches(output: &[u8], expected_version: &str) -> bool {
    let output = String::from_utf8_lossy(output);
    output.lines().any(|line| {
        let mut segments = line.split_whitespace();
        matches!(
            (segments.next(), segments.next(), segments.next()),
            (Some("Python"), Some(version), None)
                if python_version_matches_requirement(version, expected_version)
        )
    })
}

fn python_version_matches_requirement(reported_version: &str, expected_version: &str) -> bool {
    reported_version == expected_version
        || reported_version.starts_with(&format!("{expected_version}."))
}

#[cfg(test)]
mod tests {
    use super::python_version_matches_requirement;

    #[test]
    fn python_version_match_uses_component_boundaries() {
        assert!(python_version_matches_requirement("3.13.2", "3"));
        assert!(python_version_matches_requirement("3.13.2", "3.13"));
        assert!(python_version_matches_requirement("3.13.2", "3.13.2"));
        assert!(!python_version_matches_requirement("3.10.8", "3.1"));
        assert!(!python_version_matches_requirement("3.13.2", "3.13.20"));
    }
}
