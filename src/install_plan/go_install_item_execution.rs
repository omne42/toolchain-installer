use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;
use omne_process_primitives::{
    HostRecipeRequest, command_exists, command_path_exists, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{GoInstallPlanItem, GoInstallSource};

pub(crate) fn execute_go_install_item(
    item: &GoInstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    if !command_exists("go") {
        return Err(OperationError::install("go command not found"));
    }
    let destination = managed_dir.join(format!(
        "{}{}",
        item.binary_name,
        executable_suffix_for_target(target_triple)
    ));
    let env = vec![("GOBIN".to_string(), managed_dir.display().to_string())];
    let backup = InstalledBinaryBackup::stash(&destination).map_err(OperationError::install)?;
    let resolved_package = match &item.source {
        GoInstallSource::LocalPath(package_path) => {
            if !package_path.exists() {
                return Err(OperationError::install(format!(
                    "go_install local path does not exist: {}",
                    package_path.display()
                )));
            }
            if !package_path.is_dir() {
                return Err(OperationError::install(format!(
                    "go_install local path must be a directory: {}",
                    package_path.display()
                )));
            }
            let args = vec!["install".to_string(), ".".to_string()];
            if let Err(err) = run_host_recipe(
                &HostRecipeRequest::new("go".as_ref(), &args)
                    .with_env(&env)
                    .with_working_directory(package_path),
            ) {
                backup.restore().map_err(OperationError::install)?;
                return Err(OperationError::from_host_recipe(err));
            }
            package_path.display().to_string()
        }
        GoInstallSource::PackageSpec(package) => {
            let args = vec!["install".to_string(), package.clone()];
            if let Err(err) =
                run_host_recipe(&HostRecipeRequest::new("go".as_ref(), &args).with_env(&env))
            {
                backup.restore().map_err(OperationError::install)?;
                return Err(OperationError::from_host_recipe(err));
            }
            package.clone()
        }
    };

    if !command_path_exists(&destination) {
        backup.restore().map_err(OperationError::install)?;
        return Err(OperationError::install(format!(
            "expected go_install binary at {}",
            destination.display()
        )));
    }
    backup.discard().map_err(OperationError::install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(format!("go:install:{resolved_package}")),
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
                "cannot stage existing go_install binary backup {}",
                backup.display()
            ));
        }
        std::fs::rename(original, &backup).map_err(|err| {
            format!(
                "cannot stage existing go_install binary {} before reinstall: {err}",
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
                    "cannot remove failed go_install binary {} before restore: {err}",
                    self.original.display()
                )
            })?;
        }
        std::fs::rename(backup, &self.original).map_err(|err| {
            format!(
                "cannot restore previous go_install binary {} from {}: {err}",
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
                "cannot remove staged go_install binary backup {}: {err}",
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
    use crate::plan_items::{GoInstallPlanItem, GoInstallSource};

    #[cfg_attr(windows, ignore = "mock go executable is unix-specific")]
    #[test]
    fn go_install_rejects_stale_binary_when_install_creates_nothing() {
        let _guard = path_lock().lock().expect("path lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let fake_bin_dir = temp.path().join("fake-bin");
        std::fs::create_dir_all(&fake_bin_dir).expect("create fake bin dir");
        write_executable(&fake_bin_dir.join("go"), "#!/bin/sh\nexit 0\n");
        let _path_guard = ScopedPath::prepend(&fake_bin_dir);

        let managed_dir = temp.path().join("managed");
        std::fs::create_dir_all(&managed_dir).expect("create managed dir");
        let destination = managed_dir.join("stale");
        std::fs::write(&destination, "stale").expect("write stale binary");
        make_executable(&destination);

        let err = execute_go_install_item(
            &GoInstallPlanItem {
                id: "demo".to_string(),
                source: GoInstallSource::PackageSpec("example.com/demo@latest".to_string()),
                binary_name: "stale".to_string(),
            },
            host_target_triple(),
            &managed_dir,
        )
        .expect_err("stale go binary should not count as a fresh install");

        assert!(err.to_string().contains("expected go_install binary"));
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
