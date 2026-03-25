use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use omne_host_info_primitives::executable_suffix_for_target;

pub(super) fn find_managed_python_executable(
    managed_dir: &Path,
    version: &str,
    target_triple: &str,
) -> Option<PathBuf> {
    let major_minor = python_major_minor(version)?;
    let ext = executable_suffix_for_target(target_triple);
    let preferred = [
        format!("python{major_minor}{ext}"),
        format!("python3{ext}"),
        format!("python{ext}"),
    ];
    for name in preferred {
        let candidate = managed_dir.join(name);
        if executable_reports_python_version(&candidate, version) {
            return Some(candidate);
        }
    }

    let entries = std::fs::read_dir(managed_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("python") {
            continue;
        }
        if executable_reports_python_version(&path, version) {
            return Some(path);
        }
    }
    None
}

fn python_major_minor(version: &str) -> Option<String> {
    let mut segments = version.split('.');
    let major = segments.next()?.trim();
    let minor = segments.next()?.trim();
    if major.is_empty() || minor.is_empty() {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

fn executable_reports_python_version(path: &Path, version: &str) -> bool {
    if !path.exists() {
        return false;
    }
    let output = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    combined.contains(version)
}
