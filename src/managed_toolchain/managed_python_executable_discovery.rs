use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use omne_host_info_primitives::executable_suffix_for_target;

use super::managed_environment_layout::managed_python_installation_dir;

pub(crate) fn find_managed_python_executable(
    managed_dir: &Path,
    version: &str,
    target_triple: &str,
) -> Option<PathBuf> {
    let ext = executable_suffix_for_target(target_triple);
    for name in preferred_python_candidate_names(version, ext) {
        let candidate = managed_dir.join(name);
        if executable_reports_python_version(&candidate, version) {
            return Some(candidate);
        }
    }

    if let Ok(entries) = std::fs::read_dir(managed_dir) {
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
    }

    find_python_under_installation_dir(
        &managed_python_installation_dir(managed_dir),
        version,
        target_triple,
    )
}

fn find_python_under_installation_dir(
    installation_dir: &Path,
    version: &str,
    target_triple: &str,
) -> Option<PathBuf> {
    if !installation_dir.is_dir() {
        return None;
    }

    let ext = executable_suffix_for_target(target_triple);
    let mut best_match: Option<PathBuf> = None;
    let mut stack = vec![installation_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with("python") || !name.ends_with(ext) {
                continue;
            }
            if !executable_reports_python_version(&path, version) {
                continue;
            }
            if best_match.as_ref().is_none_or(|current| path < *current) {
                best_match = Some(path);
            }
        }
    }

    best_match
}

fn preferred_python_candidate_names(version: &str, ext: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(major_minor) = python_major_minor(version) {
        names.push(format!("python{major_minor}{ext}"));
    }
    if let Some(major) = python_major(version) {
        names.push(format!("python{major}{ext}"));
    }
    names.push(format!("python3{ext}"));
    names.push(format!("python{ext}"));
    names.dedup();
    names
}

fn python_major(version: &str) -> Option<&str> {
    let major = version.split('.').next()?.trim();
    (!major.is_empty()).then_some(major)
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
    python_version_output_matches(&output.stdout, version)
        || python_version_output_matches(&output.stderr, version)
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
