use std::path::{Path, PathBuf};

use omne_host_info_primitives::resolve_home_dir;

pub(crate) fn resolve_managed_toolchain_dir(
    override_dir: Option<&Path>,
    target_triple: &str,
) -> Option<PathBuf> {
    if let Some(override_dir) = override_dir {
        return Some(override_dir.to_path_buf());
    }
    if let Some(raw) =
        std::env::var_os("TOOLCHAIN_INSTALLER_MANAGED_DIR").filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(raw));
    }
    if let Some(omne_data_dir) = std::env::var_os("OMNE_DATA_DIR").filter(|value| !value.is_empty())
    {
        return Some(default_managed_dir_under_data_root(
            Path::new(&omne_data_dir),
            target_triple,
        ));
    }
    let home = resolve_home_dir()?;
    Some(default_managed_dir_under_data_root(
        &home.join(".omne_data"),
        target_triple,
    ))
}

pub(crate) fn default_managed_dir_under_data_root(
    data_root: &Path,
    target_triple: &str,
) -> PathBuf {
    data_root.join("toolchain").join(target_triple).join("bin")
}
