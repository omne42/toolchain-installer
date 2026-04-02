use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::managed_environment_layout::{managed_python_installation_dir, validated_binary_suffix};
use super::version_probe::python_binary_version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    modified: Option<SystemTime>,
    len: u64,
}

#[cfg(test)]
pub(crate) fn find_managed_python_executable(
    managed_dir: &Path,
    version: &str,
    target_triple: &str,
) -> Option<PathBuf> {
    select_managed_python_executable(managed_dir, version, target_triple, |_| true)
        .map(|candidate| candidate.path)
}

pub(crate) fn capture_managed_python_installation_state(
    managed_dir: &Path,
    target_triple: &str,
) -> HashMap<PathBuf, Option<FileFingerprint>> {
    discover_managed_python_candidate_paths(managed_dir, target_triple)
        .into_iter()
        .map(|path| {
            let fingerprint = file_fingerprint(&path);
            (path, fingerprint)
        })
        .collect()
}

pub(crate) fn find_updated_managed_python_executable(
    managed_dir: &Path,
    version: &str,
    target_triple: &str,
    preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>,
) -> Option<PathBuf> {
    select_managed_python_executable(managed_dir, version, target_triple, |candidate| {
        path_changed(preinstall_state, candidate)
    })
    .map(|candidate| candidate.path)
}

fn select_managed_python_executable<F>(
    managed_dir: &Path,
    version: &str,
    target_triple: &str,
    include_candidate: F,
) -> Option<PythonCandidate>
where
    F: Fn(&PythonCandidate) -> bool,
{
    let ext = validated_binary_suffix(target_triple);
    let mut best_match = None;
    for name in preferred_python_candidate_names(version, ext) {
        let candidate = managed_dir.join(name);
        if let Some(matched_candidate) = matched_python_candidate(&candidate, version, 0)
            && include_candidate(&matched_candidate)
        {
            record_better_match(&mut best_match, matched_candidate);
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
            if let Some(matched_candidate) = matched_python_candidate(&path, version, 1)
                && include_candidate(&matched_candidate)
            {
                record_better_match(&mut best_match, matched_candidate);
            }
        }
    }

    if let Some(matched_candidate) = find_python_under_installation_dir(
        &managed_python_installation_dir(managed_dir),
        version,
        target_triple,
    )
    .filter(&include_candidate)
    {
        record_better_match(&mut best_match, matched_candidate);
    }

    best_match
}

fn find_python_under_installation_dir(
    installation_dir: &Path,
    version: &str,
    target_triple: &str,
) -> Option<PythonCandidate> {
    if !installation_dir.is_dir() {
        return None;
    }

    let ext = validated_binary_suffix(target_triple);
    let mut best_match = None;
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
            if let Some(matched_candidate) = matched_python_candidate(&path, version, 2) {
                record_better_match(&mut best_match, matched_candidate);
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

fn matched_python_candidate(
    path: &Path,
    expected_version: &str,
    priority: u8,
) -> Option<PythonCandidate> {
    let version = probe_python_version(path)?;
    python_version_matches_requirement(&version, expected_version).then(|| PythonCandidate {
        fingerprint: file_fingerprint(path),
        path: path.to_path_buf(),
        priority,
        version,
    })
}

fn probe_python_version(path: &Path) -> Option<PythonVersion> {
    python_binary_version(path).and_then(|version| PythonVersion::parse(&version))
}

fn python_version_matches_requirement(
    reported_version: &PythonVersion,
    expected_version: &str,
) -> bool {
    let Some(expected) = PythonVersionReq::parse(expected_version) else {
        return false;
    };
    reported_version.matches(&expected)
}

fn record_better_match(best_match: &mut Option<PythonCandidate>, candidate: PythonCandidate) {
    if best_match
        .as_ref()
        .is_none_or(|current| candidate.is_better_than(current))
    {
        *best_match = Some(candidate);
    }
}

#[derive(Debug, Clone)]
struct PythonCandidate {
    fingerprint: Option<FileFingerprint>,
    path: PathBuf,
    version: PythonVersion,
    priority: u8,
}

impl PythonCandidate {
    fn is_better_than(&self, other: &Self) -> bool {
        self.version
            .cmp(&other.version)
            .then_with(|| other.priority.cmp(&self.priority))
            .then_with(|| other.path.cmp(&self.path))
            .is_gt()
    }
}

fn path_changed(
    preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>,
    candidate: &PythonCandidate,
) -> bool {
    let Some(current) = candidate.fingerprint.as_ref() else {
        return false;
    };
    match preinstall_state.get(&candidate.path) {
        Some(Some(previous)) => previous != current,
        Some(None) | None => true,
    }
}

fn discover_managed_python_candidate_paths(
    managed_dir: &Path,
    target_triple: &str,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(managed_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with("python") {
                paths.push(path);
            }
        }
    }
    paths.extend(discover_python_paths_under_installation_dir(
        &managed_python_installation_dir(managed_dir),
        target_triple,
    ));
    paths.sort();
    paths.dedup();
    paths
}

fn discover_python_paths_under_installation_dir(
    installation_dir: &Path,
    target_triple: &str,
) -> Vec<PathBuf> {
    if !installation_dir.is_dir() {
        return Vec::new();
    }

    let ext = validated_binary_suffix(target_triple);
    let mut paths = Vec::new();
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
            if name.starts_with("python") && name.ends_with(ext) {
                paths.push(path);
            }
        }
    }
    paths
}

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileFingerprint {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PythonVersion {
    major: u64,
    minor: u64,
    patch: Option<u64>,
}

impl PythonVersion {
    fn parse(raw: &str) -> Option<Self> {
        let mut segments = raw.trim().split('.');
        let major = segments.next()?.parse().ok()?;
        let minor = segments.next()?.parse().ok()?;
        let patch = segments.next().map(str::parse).transpose().ok()?;
        if segments.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    fn matches(&self, requirement: &PythonVersionReq) -> bool {
        if self.major != requirement.major {
            return false;
        }
        if let Some(minor) = requirement.minor
            && self.minor != minor
        {
            return false;
        }
        if let Some(patch) = requirement.patch {
            return self.patch == Some(patch);
        }
        true
    }
}

impl Ord for PythonVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch.unwrap_or(0)).cmp(&(
            other.major,
            other.minor,
            other.patch.unwrap_or(0),
        ))
    }
}

impl PartialOrd for PythonVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PythonVersionReq {
    major: u64,
    minor: Option<u64>,
    patch: Option<u64>,
}

impl PythonVersionReq {
    fn parse(raw: &str) -> Option<Self> {
        let mut segments = raw.trim().split('.');
        let major = segments.next()?.parse().ok()?;
        let minor = segments.next().map(str::parse).transpose().ok()?;
        let patch = segments.next().map(str::parse).transpose().ok()?;
        if segments.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}
