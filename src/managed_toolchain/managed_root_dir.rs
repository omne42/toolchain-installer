use std::path::{Path, PathBuf};

use omne_host_info_primitives::resolve_home_dir;

pub(crate) fn resolve_managed_toolchain_dir(
    override_dir: Option<&Path>,
    target_triple: &str,
) -> Option<PathBuf> {
    if let Some(override_dir) = override_dir {
        return Some(override_dir.to_path_buf());
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    use omne_host_info_primitives::resolve_home_dir;

    use super::{default_managed_dir_under_data_root, resolve_managed_toolchain_dir};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_env_var(name: &str, previous: Option<OsString>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(name, value) },
            None => unsafe { std::env::remove_var(name) },
        }
    }

    #[test]
    fn resolve_managed_toolchain_dir_ignores_process_environment_without_override() {
        let _guard = env_lock().lock().expect("env lock");
        let names = ["TOOLCHAIN_INSTALLER_MANAGED_DIR", "OMNE_DATA_DIR"];
        let previous = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_MANAGED_DIR",
                "/tmp/env-managed-toolchain",
            );
            std::env::set_var("OMNE_DATA_DIR", "/tmp/env-omne-data");
        }

        let resolved =
            resolve_managed_toolchain_dir(None, "x86_64-unknown-linux-gnu").expect("managed dir");

        for (name, value) in previous {
            restore_env_var(name, value);
        }

        let home = resolve_home_dir().expect("home");
        assert_eq!(
            resolved,
            default_managed_dir_under_data_root(
                &home.join(".omne_data"),
                "x86_64-unknown-linux-gnu",
            )
        );
    }
}
