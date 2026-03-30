use std::path::{Path, PathBuf};

pub(crate) struct ManagedDestinationBackup {
    original: PathBuf,
    backup: Option<PathBuf>,
    label: &'static str,
}

impl ManagedDestinationBackup {
    pub(crate) fn stash(original: &Path, label: &'static str) -> Result<Self, String> {
        if !original.exists() {
            return Ok(Self {
                original: original.to_path_buf(),
                backup: None,
                label,
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
                "cannot stage existing {} backup {}",
                label,
                backup.display()
            ));
        }
        std::fs::rename(original, &backup).map_err(|err| {
            format!(
                "cannot stage existing {} {} before reinstall: {err}",
                label,
                original.display()
            )
        })?;
        Ok(Self {
            original: original.to_path_buf(),
            backup: Some(backup),
            label,
        })
    }

    pub(crate) fn restore(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        remove_path_if_exists(&self.original).map_err(|err| {
            format!(
                "cannot remove failed {} {} before restore: {err}",
                self.label,
                self.original.display()
            )
        })?;
        std::fs::rename(backup, &self.original).map_err(|err| {
            format!(
                "cannot restore previous {} {} from {}: {err}",
                self.label,
                self.original.display(),
                backup.display()
            )
        })
    }

    pub(crate) fn discard(&self) -> Result<(), String> {
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
        remove_path_if_exists(backup).map_err(|err| {
            format!(
                "cannot remove staged {} backup {}: {err}",
                self.label,
                backup.display()
            )
        })
    }
}

pub(crate) fn promote_staged_file(
    staged_file: &Path,
    destination: &Path,
    label: &str,
) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "cannot create {label} destination parent {}: {err}",
                parent.display()
            )
        })?;
    }
    remove_path_if_exists(destination).map_err(|err| {
        format!(
            "cannot remove existing {label} {}: {err}",
            destination.display()
        )
    })?;
    std::fs::rename(staged_file, destination).map_err(|err| {
        format!(
            "cannot promote staged {label} {} to {}: {err}",
            staged_file.display(),
            destination.display()
        )
    })
}

fn remove_path_if_exists(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let file_type = metadata.file_type();
    if file_type.is_dir() && !file_type.is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{ManagedDestinationBackup, promote_staged_file};

    #[test]
    fn discard_removes_directory_backup_without_leaving_residue() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        std::fs::create_dir_all(&original).expect("create original directory");
        std::fs::write(original.join("tool"), "demo").expect("write original file");

        let backup =
            ManagedDestinationBackup::stash(&original, "managed binary").expect("stash backup");
        assert!(!original.exists(), "original should be moved into backup");

        std::fs::write(&original, "replacement").expect("write replacement file");
        backup.discard().expect("discard backup");

        assert!(original.is_file(), "replacement should remain in place");
        assert!(
            !temp
                .path()
                .join("managed-tool.toolchain-installer-backup")
                .exists(),
            "directory backup should be removed"
        );
    }

    #[test]
    fn restore_replaces_failed_output_with_original_directory_backup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        std::fs::create_dir_all(&original).expect("create original directory");
        std::fs::write(original.join("tool"), "demo").expect("write original file");

        let backup =
            ManagedDestinationBackup::stash(&original, "managed binary").expect("stash backup");
        std::fs::write(&original, "broken").expect("write failed output");

        backup.restore().expect("restore backup");

        assert!(original.is_dir(), "original directory should be restored");
        assert_eq!(
            std::fs::read_to_string(original.join("tool")).expect("read restored file"),
            "demo"
        );
    }

    #[test]
    fn promote_staged_file_replaces_existing_directory_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let staged = temp.path().join("stage").join("tool");
        let destination = temp.path().join("managed").join("tool");
        std::fs::create_dir_all(staged.parent().expect("stage parent")).expect("create stage");
        std::fs::write(&staged, "replacement").expect("write staged file");
        std::fs::create_dir_all(&destination).expect("create conflicting directory");
        std::fs::write(destination.join("old"), "old").expect("write old file");

        promote_staged_file(&staged, &destination, "managed binary").expect("promote");

        assert_eq!(
            std::fs::read_to_string(&destination).expect("read promoted file"),
            "replacement"
        );
    }

    #[test]
    fn stash_without_existing_path_keeps_backup_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("missing");

        let backup =
            ManagedDestinationBackup::stash(&original, "managed binary").expect("stash backup");

        backup.discard().expect("discard noop");
        backup.restore().expect("restore noop");
        assert!(!original.exists());
    }
}
