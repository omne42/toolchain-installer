use std::ffi::OsString;
use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

pub(crate) fn validated_binary_suffix(target_triple: &str) -> &'static str {
    executable_suffix_for_target(target_triple)
}

pub(crate) fn managed_uv_binary_path(target_triple: &str, managed_dir: &Path) -> PathBuf {
    managed_dir.join(format!("uv{}", validated_binary_suffix(target_triple)))
}

pub(crate) fn bootstrap_uv_root(managed_dir: &Path) -> PathBuf {
    managed_dir.join(".uv-bootstrap")
}

pub(crate) fn bootstrap_uv_binary_path(target_triple: &str, bootstrap_root: &Path) -> PathBuf {
    let binary_name = format!("uv{}", validated_binary_suffix(target_triple));
    if target_triple.contains("windows") {
        return bootstrap_root.join("Scripts").join(binary_name);
    }
    bootstrap_root.join("bin").join(binary_name)
}

pub(crate) fn bootstrap_uv_site_packages_dir(bootstrap_root: &Path) -> PathBuf {
    bootstrap_root.join("site-packages")
}

pub(crate) fn managed_tool_binary_path(
    executable_name: &str,
    target_triple: &str,
    managed_dir: &Path,
) -> PathBuf {
    managed_dir.join(format!(
        "{executable_name}{}",
        validated_binary_suffix(target_triple)
    ))
}

pub(crate) fn managed_python_installation_dir(managed_dir: &Path) -> PathBuf {
    managed_dir.join(".uv-python")
}

pub(crate) fn managed_uv_tool_dir(managed_dir: &Path) -> PathBuf {
    managed_dir.join(".uv-tools")
}

pub(crate) fn managed_uv_cache_dir(managed_dir: &Path) -> PathBuf {
    managed_dir.join(".uv-cache")
}

pub(crate) fn managed_python_shim_paths(
    version: &str,
    target_triple: &str,
    managed_dir: &Path,
) -> Vec<PathBuf> {
    let ext = validated_binary_suffix(target_triple);
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
        .into_iter()
        .map(|name| managed_dir.join(name))
        .collect()
}

pub(crate) fn managed_uv_process_env(managed_dir: &Path) -> Vec<(OsString, OsString)> {
    vec![
        (
            OsString::from("UV_TOOL_DIR"),
            managed_uv_tool_dir(managed_dir).into_os_string(),
        ),
        (
            OsString::from("UV_TOOL_BIN_DIR"),
            managed_dir.as_os_str().to_os_string(),
        ),
        (
            OsString::from("UV_PYTHON_INSTALL_DIR"),
            managed_python_installation_dir(managed_dir).into_os_string(),
        ),
        (
            OsString::from("UV_PYTHON_BIN_DIR"),
            managed_dir.as_os_str().to_os_string(),
        ),
        (OsString::from("UV_PYTHON_INSTALL_BIN"), OsString::from("1")),
        (OsString::from("UV_MANAGED_PYTHON"), OsString::from("1")),
        (
            OsString::from("UV_CACHE_DIR"),
            managed_uv_cache_dir(managed_dir).into_os_string(),
        ),
    ]
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

#[cfg(test)]
mod tests {
    use super::{managed_uv_cache_dir, managed_uv_process_env};

    #[cfg(unix)]
    #[test]
    fn managed_uv_process_env_preserves_non_utf8_managed_dir_bytes() {
        use std::ffi::{OsStr, OsString};
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        let managed_dir = Path::new(OsStr::from_bytes(b"/tmp/toolchain-installer-\xFF-managed"));
        let env = managed_uv_process_env(managed_dir);

        let tool_bin_dir = env
            .iter()
            .find(|(name, _)| name == &OsString::from("UV_TOOL_BIN_DIR"))
            .map(|(_, value)| value)
            .expect("UV_TOOL_BIN_DIR env");
        assert_eq!(tool_bin_dir.as_bytes(), managed_dir.as_os_str().as_bytes());

        let cache_dir = env
            .iter()
            .find(|(name, _)| name == &OsString::from("UV_CACHE_DIR"))
            .map(|(_, value)| value)
            .expect("UV_CACHE_DIR env");
        assert_eq!(
            cache_dir.as_bytes(),
            managed_uv_cache_dir(managed_dir).as_os_str().as_bytes()
        );
    }
}
