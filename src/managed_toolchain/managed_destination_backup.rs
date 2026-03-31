use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct ManagedDestinationBackup {
    original: PathBuf,
    backup: Option<PathBuf>,
    label: &'static str,
}

impl ManagedDestinationBackup {
    pub(crate) fn stash(original: &Path, label: &'static str) -> Result<Self, String> {
        let backup = destination_backup_path(original);
        reconcile_backup_before_stash(original, &backup, label)?;
        if !original.exists() {
            return Ok(Self {
                original: original.to_path_buf(),
                backup: None,
                label,
            });
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

    pub(crate) fn discard_with_warning(&self) -> Option<String> {
        let err = self.discard().err()?;
        let Some(backup) = self.backup.as_ref() else {
            return Some(format!(
                "{} installed at {} but cleanup warning: {err}",
                self.label,
                self.original.display()
            ));
        };
        if !backup.exists() {
            return Some(format!(
                "{} installed at {} but cleanup warning: {err}",
                self.label,
                self.original.display()
            ));
        }

        match quarantine_backup_path(backup) {
            Ok(quarantined) => Some(format!(
                "{} installed at {} but cleanup warning: {err}; moved stale backup to {}",
                self.label,
                self.original.display(),
                quarantined.display()
            )),
            Err(quarantine_err) => Some(format!(
                "{} installed at {} but cleanup warning: {err}; stale backup remains at {} ({quarantine_err})",
                self.label,
                self.original.display(),
                backup.display()
            )),
        }
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

fn reconcile_backup_before_stash(
    original: &Path,
    backup: &Path,
    label: &str,
) -> Result<(), String> {
    if !backup.exists() {
        return Ok(());
    }
    if !original.exists() {
        std::fs::rename(backup, original).map_err(|err| {
            format!(
                "cannot restore interrupted {} {} from stale backup {} before reinstall: {err}",
                label,
                original.display(),
                backup.display()
            )
        })?;
        return Ok(());
    }

    let quarantined = quarantine_backup_path(backup).map_err(|err| {
        format!(
            "cannot move stale {} backup {} aside before reinstall: {err}",
            label,
            backup.display()
        )
    })?;
    debug_assert!(
        quarantined.exists(),
        "quarantined stale backup should exist after rename"
    );
    Ok(())
}

fn destination_backup_path(destination: &Path) -> PathBuf {
    destination.with_file_name(format!(
        "{}.toolchain-installer-backup",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("managed-tool")
    ))
}

fn quarantine_backup_path(backup_path: &Path) -> Result<PathBuf, String> {
    let parent = backup_path
        .parent()
        .ok_or_else(|| format!("cannot resolve parent for backup {}", backup_path.display()))?;
    let file_name = backup_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("managed-tool.toolchain-installer-backup");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let quarantined = parent.join(format!("{file_name}.stale-{}-{nonce}", std::process::id()));
    std::fs::rename(backup_path, &quarantined).map_err(|err| {
        format!(
            "cannot move stale backup {} aside to {}: {err}",
            backup_path.display(),
            quarantined.display()
        )
    })?;
    Ok(quarantined)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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

    #[test]
    fn stash_restores_interrupted_backup_before_restaging() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        std::fs::write(&original, "original").expect("write original");

        let first = ManagedDestinationBackup::stash(&original, "managed binary").expect("stash");
        assert!(!original.exists(), "original should be staged away");
        drop(first);

        let second = ManagedDestinationBackup::stash(&original, "managed binary").expect("restash");
        assert!(
            !original.exists(),
            "original should be restaged on second attempt"
        );
        second.restore().expect("restore after second attempt");

        assert_eq!(
            std::fs::read_to_string(&original).expect("read restored original"),
            "original"
        );
    }

    #[test]
    fn stash_quarantines_stale_backup_when_destination_already_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        std::fs::write(&original, "current").expect("write current");
        let backup_path = temp.path().join("managed-tool.toolchain-installer-backup");
        std::fs::write(&backup_path, "stale").expect("write stale backup");

        let backup =
            ManagedDestinationBackup::stash(&original, "managed binary").expect("stash backup");
        assert!(!original.exists(), "original should be staged away");
        assert!(
            backup_path.exists(),
            "canonical backup path should be reused after stale backup quarantine"
        );
        backup.restore().expect("restore");

        let quarantined = std::fs::read_dir(temp.path())
            .expect("read temp")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.contains(".toolchain-installer-backup.stale-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            quarantined.len(),
            1,
            "expected one quarantined stale backup"
        );
        assert_eq!(
            std::fs::read_to_string(&original).expect("read restored original"),
            "current"
        );
    }

    #[cfg(unix)]
    #[test]
    fn discard_with_warning_reports_stale_backup_when_cleanup_cannot_quarantine() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        #[cfg(target_os = "linux")]
        let protected_backup = PathBuf::from("/proc");
        #[cfg(target_os = "macos")]
        let protected_backup = PathBuf::from("/System");
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let protected_backup = PathBuf::from("/dev");
        let backup = ManagedDestinationBackup {
            original: original.clone(),
            backup: Some(protected_backup.clone()),
            label: "managed binary",
        };

        let detail = backup
            .discard_with_warning()
            .expect("discard warning should be reported");

        assert!(detail.contains("cleanup warning"));
        assert!(detail.contains(&format!(
            "stale backup remains at {}",
            protected_backup.display()
        )));
        assert!(detail.contains(&original.display().to_string()));
    }
}
