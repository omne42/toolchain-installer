use std::path::{Path, PathBuf};

use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{CargoInstallPlanItem, CargoInstallSource};

use super::item_destination_resolution::{cargo_install_root, resolve_cargo_install_destination};

pub(crate) fn execute_cargo_install_item(
    item: &CargoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("cargo") {
        return Err(OperationError::install("cargo command not found"));
    }
    let install_root = cargo_install_root(managed_dir);
    let destination = resolve_cargo_install_destination(item, target_triple, managed_dir);

    let mut args = vec!["install".to_string()];
    let source = match &item.source {
        CargoInstallSource::LocalPath(package_path) => {
            if !package_path.exists() {
                return Err(OperationError::install(format!(
                    "cargo_install local path does not exist: {}",
                    package_path.display()
                )));
            }
            if !package_path.is_dir() {
                return Err(OperationError::install(format!(
                    "cargo_install local path must be a directory: {}",
                    package_path.display()
                )));
            }
            args.push("--path".to_string());
            args.push(package_path.display().to_string());
            format!("cargo:path:{}", package_path.display())
        }
        CargoInstallSource::RegistryPackage { package, version } => {
            args.push("--locked".to_string());
            args.push(package.clone());
            if let Some(version) = version.as_deref() {
                args.push("--version".to_string());
                args.push(version.to_string());
            }
            format!("cargo:crate:{package}")
        }
    };
    args.push("--root".to_string());
    args.push(install_root.display().to_string());
    let backup = InstalledBinaryBackup::stash(&destination).map_err(OperationError::install)?;
    if let Err(err) = run_host_recipe(&HostRecipeRequest::new("cargo".as_ref(), &args)) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::from_host_recipe(err));
    }

    if !command_path_exists(&destination) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(format!(
            "expected cargo_install binary at {}",
            destination.display()
        )));
    }
    backup.discard().map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(source),
        source_kind: None,
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

struct InstalledBinaryBackup {
    original: PathBuf,
    backup: Option<PathBuf>,
}

impl InstalledBinaryBackup {
    fn stash(original: &Path) -> Result<Self, String> {
        if !original.exists() {
            return Ok(Self {
                original: original.to_path_buf(),
                backup: None,
            });
        }

        let backup = original.with_file_name(format!(
            "{}.toolchain-installer-backup",
            original
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("managed-tool")
        ));
        if backup.exists() {
            return Err(format!(
                "cannot stage existing cargo_install binary backup {}",
                backup.display()
            ));
        }
        std::fs::rename(original, &backup).map_err(|err| {
            format!(
                "cannot stage existing cargo_install binary {} before reinstall: {err}",
                original.display()
            )
        })?;
        Ok(Self {
            original: original.to_path_buf(),
            backup: Some(backup),
        })
    }

    fn restore(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        if self.original.exists() {
            std::fs::remove_file(&self.original).map_err(|err| {
                format!(
                    "cannot remove failed cargo_install binary {} before restore: {err}",
                    self.original.display()
                )
            })?;
        }
        std::fs::rename(backup, &self.original).map_err(|err| {
            format!(
                "cannot restore previous cargo_install binary {} from {}: {err}",
                self.original.display(),
                backup.display()
            )
        })
    }

    fn discard(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        std::fs::remove_file(backup).map_err(|err| {
            format!(
                "cannot remove staged cargo_install binary backup {}: {err}",
                backup.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    use super::*;
    use crate::plan_items::{CargoInstallPlanItem, CargoInstallSource};

    #[cfg_attr(windows, ignore = "mock cargo executable is unix-specific")]
    #[test]
    fn cargo_install_rejects_stale_binary_when_install_creates_nothing() {
        let _guard = path_lock().lock().expect("path lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let fake_bin_dir = temp.path().join("fake-bin");
        std::fs::create_dir_all(&fake_bin_dir).expect("create fake bin dir");
        write_executable(&fake_bin_dir.join("cargo"), "#!/bin/sh\nexit 0\n");
        let _path_guard = ScopedPath::prepend(&fake_bin_dir);

        let managed_dir = temp.path().join("managed");
        let destination = cargo_install_root(&managed_dir).join("bin").join("stale");
        std::fs::create_dir_all(destination.parent().expect("destination parent"))
            .expect("create destination parent");
        std::fs::write(&destination, "stale").expect("write stale binary");
        make_executable(&destination);

        let err = execute_cargo_install_item(
            &CargoInstallPlanItem {
                id: "demo".to_string(),
                source: CargoInstallSource::RegistryPackage {
                    package: "demo".to_string(),
                    version: None,
                },
                binary_name: "stale".to_string(),
            },
            host_target_triple(),
            &managed_dir,
        )
        .expect_err("stale cargo binary should not count as a fresh install");

        assert!(err.to_string().contains("expected cargo_install binary"));
        assert_eq!(
            std::fs::read_to_string(&destination).expect("restored stale binary"),
            "stale"
        );
        assert!(
            !destination
                .with_file_name("stale.toolchain-installer-backup")
                .exists(),
            "staged backup should be cleaned up after restore"
        );
    }

    fn path_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn host_target_triple() -> &'static str {
        #[cfg(windows)]
        {
            "x86_64-pc-windows-msvc"
        }
        #[cfg(target_os = "macos")]
        {
            "x86_64-apple-darwin"
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            "x86_64-unknown-linux-gnu"
        }
    }

    struct ScopedPath {
        original: Option<OsString>,
    }

    impl ScopedPath {
        fn prepend(path: &Path) -> Self {
            let original = std::env::var_os("PATH");
            let mut paths = vec![path.to_path_buf()];
            if let Some(existing) = original.as_ref() {
                paths.extend(std::env::split_paths(existing));
            }
            let joined = std::env::join_paths(paths).expect("join PATH");
            unsafe {
                std::env::set_var("PATH", joined);
            }
            Self { original }
        }
    }

    impl Drop for ScopedPath {
        fn drop(&mut self) {
            match self.original.as_ref() {
                Some(original) => unsafe {
                    std::env::set_var("PATH", original);
                },
                None => unsafe {
                    std::env::remove_var("PATH");
                },
            }
        }
    }

    fn write_executable(path: &Path, contents: &str) {
        std::fs::write(path, contents).expect("write executable");
        make_executable(path);
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod executable");
        }
    }
}
