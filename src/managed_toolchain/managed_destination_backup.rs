use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use omne_fs_primitives::{AdvisoryLockGuard, lock_advisory_file_in_ambient_root};

pub(crate) struct ManagedDestinationBackup {
    original: PathBuf,
    backup: Option<PathBuf>,
    label: &'static str,
    _lock: AdvisoryLockGuard,
}

impl ManagedDestinationBackup {
    pub(crate) fn stash(original: &Path, label: &'static str) -> Result<Self, String> {
        let lock = acquire_destination_lock(original, label)?;
        let backup = destination_backup_path(original);
        reconcile_backup_before_stash(original, &backup, label)?;
        if !path_entry_exists(original) {
            return Ok(Self {
                original: original.to_path_buf(),
                backup: None,
                label,
                _lock: lock,
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
            _lock: lock,
        })
    }

    pub(crate) fn restore(&self) -> Result<(), String> {
        remove_path_if_exists(&self.original).map_err(|err| {
            format!(
                "cannot remove failed {} {} before restore: {err}",
                self.label,
                self.original.display()
            )
        })?;
        let Some(backup) = self.backup.as_ref() else {
            return Ok(());
        };
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
        if !path_entry_exists(backup) {
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

pub(crate) fn lock_managed_destination(
    destination: &Path,
    label: &str,
) -> Result<AdvisoryLockGuard, String> {
    acquire_destination_lock(destination, label)
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

fn path_entry_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn reconcile_backup_before_stash(
    original: &Path,
    backup: &Path,
    label: &str,
) -> Result<(), String> {
    if !path_entry_exists(backup) {
        return Ok(());
    }
    if !path_entry_exists(original) {
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
        path_entry_exists(&quarantined),
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

fn acquire_destination_lock(destination: &Path, label: &str) -> Result<AdvisoryLockGuard, String> {
    let lock_root = destination.parent().unwrap_or_else(|| Path::new("."));
    let lock_file = destination_lock_file_name(destination);
    lock_advisory_file_in_ambient_root(
        lock_root,
        label,
        &lock_file,
        "managed destination lock file",
    )
    .map_err(|err| {
        format!(
            "cannot lock {} {} before reinstall: {err}",
            label,
            destination.display()
        )
    })
}

fn destination_lock_file_name(destination: &Path) -> PathBuf {
    let label = destination
        .file_name()
        .map(|name| sanitize_lock_component(&name.to_string_lossy()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "managed-tool".to_string());
    PathBuf::from(format!(".toolchain-installer-lock-{label}.lock"))
}

fn sanitize_lock_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
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
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::{ManagedDestinationBackup, path_entry_exists, promote_staged_file};

    #[test]
    fn restore_removes_failed_output_when_original_did_not_exist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("python3.13");
        let backup =
            ManagedDestinationBackup::stash(&destination, "managed binary").expect("stash backup");

        std::fs::write(&destination, b"failed install artifact").expect("write failed artifact");
        assert!(
            destination.exists(),
            "failed artifact should exist before restore"
        );

        backup.restore().expect("restore failed artifact");

        assert!(
            !destination.exists(),
            "restore should remove failed output when no original existed"
        );
    }

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
    fn stash_quarantines_dangling_symlink_backup_when_destination_already_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        std::fs::write(&original, "current").expect("write current");
        let backup_path = temp.path().join("managed-tool.toolchain-installer-backup");
        symlink(temp.path().join("missing-target"), &backup_path).expect("write dangling symlink");

        let backup =
            ManagedDestinationBackup::stash(&original, "managed binary").expect("stash backup");
        assert!(!original.exists(), "original should be staged away");
        assert!(
            path_entry_exists(&backup_path),
            "canonical backup path should be reused after dangling backup quarantine"
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
            "expected one quarantined dangling stale backup"
        );
        assert_eq!(
            std::fs::read_to_string(&original).expect("read restored original"),
            "current"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stash_treats_dangling_destination_symlink_as_existing_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        symlink(temp.path().join("missing-target"), &original).expect("write dangling symlink");

        let backup =
            ManagedDestinationBackup::stash(&original, "managed binary").expect("stash backup");

        assert!(
            !path_entry_exists(&original),
            "original symlink should be moved into canonical backup"
        );
        let backup_path = temp.path().join("managed-tool.toolchain-installer-backup");
        assert!(
            path_entry_exists(&backup_path),
            "canonical backup should preserve the dangling destination entry"
        );

        backup.restore().expect("restore symlink");

        assert!(
            path_entry_exists(&original),
            "restored original should keep the dangling symlink entry"
        );
        assert!(
            std::fs::symlink_metadata(&original)
                .expect("metadata")
                .file_type()
                .is_symlink(),
            "restored original should remain a symlink"
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
            _lock: super::acquire_destination_lock(&original, "managed binary").expect("lock"),
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

    #[test]
    fn stash_serializes_same_destination_until_first_guard_drops() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("managed-tool");
        std::fs::write(&original, "original").expect("write original");

        let first = ManagedDestinationBackup::stash(&original, "managed binary").expect("stash");
        let (tx, rx) = mpsc::channel();
        let original_for_thread = original.clone();
        let handle = thread::spawn(move || {
            let second = ManagedDestinationBackup::stash(&original_for_thread, "managed binary")
                .expect("second stash");
            second.restore().expect("second restore");
            tx.send(()).expect("send completion");
        });

        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "second stash should wait for the advisory lock"
        );

        first.restore().expect("restore first");
        drop(first);

        rx.recv_timeout(Duration::from_secs(2))
            .expect("second stash should complete after lock release");
        handle.join().expect("join thread");
        assert_eq!(
            std::fs::read_to_string(&original).expect("read restored original"),
            "original"
        );
    }
}
