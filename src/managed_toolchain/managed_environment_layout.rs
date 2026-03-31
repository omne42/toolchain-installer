use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

pub(crate) fn validated_binary_suffix(target_triple: &str) -> &'static str {
    executable_suffix_for_target(target_triple)
        .expect("target triple should be validated before computing managed paths")
}

pub(crate) fn managed_uv_binary_path(target_triple: &str, managed_dir: &Path) -> PathBuf {
    managed_dir.join(format!("uv{}", validated_binary_suffix(target_triple)))
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

pub(crate) fn managed_uv_process_env(managed_dir: &Path) -> Vec<(String, String)> {
    vec![
        (
            "UV_TOOL_DIR".to_string(),
            managed_dir.join(".uv-tools").display().to_string(),
        ),
        (
            "UV_TOOL_BIN_DIR".to_string(),
            managed_dir.display().to_string(),
        ),
        (
            "UV_PYTHON_INSTALL_DIR".to_string(),
            managed_python_installation_dir(managed_dir)
                .display()
                .to_string(),
        ),
        (
            "UV_PYTHON_BIN_DIR".to_string(),
            managed_dir.display().to_string(),
        ),
        ("UV_PYTHON_INSTALL_BIN".to_string(), "1".to_string()),
        ("UV_MANAGED_PYTHON".to_string(), "1".to_string()),
        (
            "UV_CACHE_DIR".to_string(),
            managed_dir.join(".uv-cache").display().to_string(),
        ),
    ]
}
