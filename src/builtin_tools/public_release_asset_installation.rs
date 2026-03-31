use std::fs;
use std::path::{Path, PathBuf};

use github_kit::GitHubReleaseAsset;
use omne_artifact_install_primitives::{
    ArchiveTreeInstallRequest, BinaryArchiveInstallRequest, InstalledArchiveBinary,
    download_and_install_archive_tree, download_and_install_binary_from_archive,
};
use omne_fs_primitives::{AtomicWriteOptions, write_file_atomically};
use omne_integrity_primitives::{Sha256Digest, parse_sha256_digest};

use crate::artifact::InstallSource;
use crate::contracts::{BootstrapArchiveFormat, BootstrapArchiveMatch};
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::external_gateway::gateway_candidate_for_git_release_asset;
use crate::github_release_metadata::fetch_latest_release_metadata;
use crate::installer_runtime_config::InstallerRuntimeConfig;
pub(crate) async fn install_gh_from_public_release(
    target_triple: &str,
    binary_ext: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    let suffix = gh_release_asset_suffix_for_target(target_triple).ok_or_else(|| {
        OperationError::install(format!(
            "gh public recipe unsupported on target `{target_triple}`"
        ))
    })?;
    let release = fetch_latest_release_metadata(client, cfg, "cli/cli").await?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(suffix))
        .ok_or_else(|| {
            OperationError::download(format!("cannot find gh release asset suffix `{suffix}`"))
        })?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref())
        .ok_or_else(|| OperationError::download("missing sha256 digest in gh release metadata"))?;
    let candidates = build_download_candidates(
        &asset.browser_download_url,
        &cfg.download_sources.mirror_prefixes,
        None,
    );
    let archive_binary_hint = target_triple
        .contains("windows")
        .then(|| format!("bin/gh{binary_ext}"));
    let downloaded = download_and_install_binary_from_archive(
        client,
        &candidates,
        &BinaryArchiveInstallRequest {
            canonical_url: &asset.browser_download_url,
            destination,
            asset_name: &asset.name,
            binary_name: &format!("gh{binary_ext}"),
            tool_name: "gh",
            archive_binary_hint: archive_binary_hint.as_deref(),
            expected_sha256: Some(&expected_sha),
            max_download_bytes: cfg.download.max_download_bytes,
        },
    )
    .await
    .map_err(OperationError::from_artifact_install)?;
    let InstalledArchiveBinary {
        source,
        archive_match,
    } = downloaded;
    Ok(InstallSource::new(
        source.url,
        result_source_kind_for_download_candidate(source.kind),
    )
    .with_archive_match(archive_match.into()))
}

pub(crate) async fn install_git_from_public_release(
    target_triple: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    let release = fetch_latest_release_metadata(client, cfg, "git-for-windows/git").await?;
    let asset = select_mingit_release_asset_for_target(&release.assets, target_triple).ok_or_else(
        || {
            OperationError::download(format!(
                "cannot find MinGit asset for target `{target_triple}`"
            ))
        },
    )?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref()).ok_or_else(|| {
        OperationError::download("missing sha256 digest in git-for-windows release metadata")
    })?;
    let gateway = gateway_candidate_for_git_release_asset(cfg, &release.tag_name, &asset.name);
    if target_triple.contains("windows") {
        return download_and_install_mingit_bundle(MingitBundleInstallRequest {
            client,
            canonical_url: &asset.browser_download_url,
            asset_name: &asset.name,
            mirror_prefixes: &cfg.download_sources.mirror_prefixes,
            gateway_candidate: gateway.as_deref(),
            destination,
            expected_sha: &expected_sha,
            max_download_bytes: cfg.download.max_download_bytes,
        })
        .await;
    }

    let candidates = build_download_candidates(
        &asset.browser_download_url,
        &cfg.download_sources.mirror_prefixes,
        gateway.as_deref(),
    );
    let downloaded = download_and_install_binary_from_archive(
        client,
        &candidates,
        &BinaryArchiveInstallRequest {
            canonical_url: &asset.browser_download_url,
            destination,
            asset_name: &asset.name,
            binary_name: "git.exe",
            tool_name: "git",
            archive_binary_hint: None,
            expected_sha256: Some(&expected_sha),
            max_download_bytes: cfg.download.max_download_bytes,
        },
    )
    .await
    .map_err(OperationError::from_artifact_install)?;
    let InstalledArchiveBinary {
        source,
        archive_match,
    } = downloaded;
    Ok(InstallSource::new(
        source.url,
        result_source_kind_for_download_candidate(source.kind),
    )
    .with_archive_match(archive_match.into()))
}

pub(crate) fn select_mingit_release_asset_for_target<'a>(
    assets: &'a [GitHubReleaseAsset],
    target_triple: &str,
) -> Option<&'a GitHubReleaseAsset> {
    match target_triple {
        "x86_64-pc-windows-msvc" => assets
            .iter()
            .find(|asset| {
                asset.name.starts_with("MinGit-") && asset.name.ends_with("-busybox-64-bit.zip")
            })
            .or_else(|| {
                assets.iter().find(|asset| {
                    asset.name.starts_with("MinGit-")
                        && asset.name.ends_with("-64-bit.zip")
                        && !asset.name.contains("busybox")
                })
            }),
        "aarch64-pc-windows-msvc" => assets
            .iter()
            .find(|asset| asset.name.starts_with("MinGit-") && asset.name.ends_with("-arm64.zip")),
        _ => None,
    }
}

pub(crate) fn gh_release_asset_suffix_for_target(target_triple: &str) -> Option<&'static str> {
    match target_triple {
        "x86_64-unknown-linux-gnu" => Some("_linux_amd64.tar.gz"),
        "aarch64-unknown-linux-gnu" => Some("_linux_arm64.tar.gz"),
        "x86_64-apple-darwin" => Some("_macOS_amd64.zip"),
        "aarch64-apple-darwin" => Some("_macOS_arm64.zip"),
        "x86_64-pc-windows-msvc" => Some("_windows_amd64.zip"),
        "aarch64-pc-windows-msvc" => Some("_windows_arm64.zip"),
        _ => None,
    }
}

struct MingitBundleInstallRequest<'a> {
    client: &'a reqwest::Client,
    canonical_url: &'a str,
    asset_name: &'a str,
    mirror_prefixes: &'a [String],
    gateway_candidate: Option<&'a str>,
    destination: &'a Path,
    expected_sha: &'a Sha256Digest,
    max_download_bytes: Option<u64>,
}

async fn download_and_install_mingit_bundle(
    request: MingitBundleInstallRequest<'_>,
) -> OperationResult<InstallSource> {
    let MingitBundleInstallRequest {
        client,
        canonical_url,
        asset_name,
        mirror_prefixes,
        gateway_candidate,
        destination,
        expected_sha,
        max_download_bytes,
    } = request;
    let managed_dir = destination.parent().ok_or_else(|| {
        OperationError::install(format!(
            "cannot determine managed dir for {}",
            destination.display()
        ))
    })?;
    let portable_root = managed_dir.join("git-portable");
    let staging_root = managed_dir.join("git-portable.stage");
    let backup_root = managed_dir.join("git-portable.backup");
    remove_dir_if_exists(&staging_root)?;

    let candidates = build_download_candidates(canonical_url, mirror_prefixes, gateway_candidate);
    let selected = download_and_install_archive_tree(
        client,
        &candidates,
        &ArchiveTreeInstallRequest {
            canonical_url,
            destination: &staging_root,
            asset_name,
            expected_sha256: Some(expected_sha),
            max_download_bytes,
        },
    )
    .await
    .map_err(OperationError::from_artifact_install)?;
    let (staged_git, matched_archive_path) = discover_mingit_executable(&staging_root)?;
    let relative_git = staged_git.strip_prefix(&staging_root).map_err(|err| {
        OperationError::install(format!("git executable not under staging dir: {err}"))
    })?;
    finalize_mingit_installation(
        &portable_root,
        &staging_root,
        &backup_root,
        destination,
        managed_dir,
        relative_git,
    )?;

    Ok(InstallSource::new(
        selected.url,
        result_source_kind_for_download_candidate(selected.kind),
    )
    .with_archive_match(BootstrapArchiveMatch {
        format: BootstrapArchiveFormat::Zip,
        path: matched_archive_path,
    }))
}

fn remove_dir_if_exists(path: &Path) -> OperationResult<()> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|err| OperationError::install(err.to_string()))?;
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> OperationResult<()> {
    if path.exists() {
        fs::remove_file(path).map_err(|err| OperationError::install(err.to_string()))?;
    }
    Ok(())
}

fn restore_backup_if_needed(current: &Path, backup: &Path) -> OperationResult<()> {
    if !backup.exists() {
        return Ok(());
    }
    if current.exists() {
        return remove_backup_path(backup);
    }
    fs::rename(backup, current).map_err(|err| OperationError::install(err.to_string()))
}

fn remove_backup_path(path: &Path) -> OperationResult<()> {
    if path.is_dir() {
        remove_dir_if_exists(path)
    } else {
        remove_file_if_exists(path)
    }
}

fn mingit_launcher_backup_path(destination: &Path) -> PathBuf {
    destination.with_file_name(format!(
        "{}.toolchain-installer-backup",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("git")
    ))
}

fn restore_mingit_transaction_failure(
    portable_root: &Path,
    backup_root: &Path,
    launcher_destination: &Path,
    launcher_backup: &Path,
) -> OperationResult<()> {
    if portable_root.exists() {
        remove_dir_if_exists(portable_root)?;
    }
    restore_backup_if_needed(portable_root, backup_root)?;

    if launcher_destination.exists() {
        remove_file_if_exists(launcher_destination)?;
    }
    restore_backup_if_needed(launcher_destination, launcher_backup)
}

struct MingitInstallationTransaction<'a> {
    portable_root: &'a Path,
    backup_root: &'a Path,
    launcher_destination: &'a Path,
    launcher_backup: &'a Path,
}

impl<'a> MingitInstallationTransaction<'a> {
    fn prepare(
        portable_root: &'a Path,
        backup_root: &'a Path,
        launcher_destination: &'a Path,
        launcher_backup: &'a Path,
    ) -> OperationResult<Self> {
        restore_backup_if_needed(portable_root, backup_root)?;
        restore_backup_if_needed(launcher_destination, launcher_backup)?;
        remove_dir_if_exists(backup_root)?;
        remove_file_if_exists(launcher_backup)?;
        if launcher_destination.exists() {
            fs::rename(launcher_destination, launcher_backup)
                .map_err(|err| OperationError::install(err.to_string()))?;
        }
        Ok(Self {
            portable_root,
            backup_root,
            launcher_destination,
            launcher_backup,
        })
    }

    fn rollback(&self) -> OperationResult<()> {
        restore_mingit_transaction_failure(
            self.portable_root,
            self.backup_root,
            self.launcher_destination,
            self.launcher_backup,
        )
    }

    fn commit(self) -> OperationResult<()> {
        remove_dir_if_exists(self.backup_root)?;
        remove_file_if_exists(self.launcher_backup)?;
        Ok(())
    }
}

pub(crate) fn replace_mingit_installation(
    portable_root: &Path,
    staging_root: &Path,
    backup_root: &Path,
) -> OperationResult<()> {
    restore_backup_if_needed(portable_root, backup_root)?;
    remove_dir_if_exists(backup_root)?;
    if portable_root.exists() {
        fs::rename(portable_root, backup_root)
            .map_err(|err| OperationError::install(err.to_string()))?;
    }

    if let Err(err) = fs::rename(staging_root, portable_root) {
        if backup_root.exists() {
            let _ = fs::rename(backup_root, portable_root);
        }
        return Err(OperationError::install(err.to_string()));
    }
    Ok(())
}

fn finalize_mingit_installation(
    portable_root: &Path,
    staging_root: &Path,
    backup_root: &Path,
    launcher_destination: &Path,
    managed_dir: &Path,
    relative_git: &Path,
) -> OperationResult<()> {
    finalize_mingit_installation_with_launcher_writer(
        portable_root,
        staging_root,
        backup_root,
        launcher_destination,
        &mingit_launcher_backup_path(launcher_destination),
        || {
            write_mingit_launcher(
                launcher_destination,
                managed_dir,
                &portable_root.join(relative_git),
            )
        },
    )
}

fn finalize_mingit_installation_with_launcher_writer<F>(
    portable_root: &Path,
    staging_root: &Path,
    backup_root: &Path,
    launcher_destination: &Path,
    launcher_backup: &Path,
    write_launcher: F,
) -> OperationResult<()>
where
    F: FnOnce() -> OperationResult<()>,
{
    let transaction = MingitInstallationTransaction::prepare(
        portable_root,
        backup_root,
        launcher_destination,
        launcher_backup,
    )?;
    if let Err(err) = replace_mingit_installation(portable_root, staging_root, backup_root) {
        let _ = restore_backup_if_needed(launcher_destination, launcher_backup);
        return Err(err);
    }
    if let Err(err) = write_launcher() {
        let _ = transaction.rollback();
        return Err(err);
    }
    transaction.commit()
}

const MINGIT_GIT_ENTRY_SUFFIXES: [&str; 4] = [
    "/cmd/git.exe",
    "/mingw64/bin/git.exe",
    "/usr/bin/git.exe",
    "/bin/git.exe",
];

fn mingit_git_entry_priority(path: &str) -> Option<usize> {
    MINGIT_GIT_ENTRY_SUFFIXES
        .iter()
        .position(|suffix| path.ends_with(suffix))
}

fn discover_mingit_executable(portable_root: &Path) -> OperationResult<(PathBuf, String)> {
    let mut best_match: Option<(usize, String, PathBuf)> = None;
    let mut stack = vec![portable_root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|err| OperationError::install(err.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|err| OperationError::install(err.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|err| OperationError::install(err.to_string()))?;
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let relative = path.strip_prefix(portable_root).map_err(|err| {
                OperationError::install(format!(
                    "portable git path is not under extracted root: {err}"
                ))
            })?;
            let normalized = relative.to_string_lossy().replace('\\', "/");
            let Some(priority) = mingit_git_entry_priority(&normalized) else {
                continue;
            };
            let should_replace = best_match
                .as_ref()
                .map(|(current_priority, current_path, _)| {
                    priority < *current_priority
                        || (priority == *current_priority && normalized < *current_path)
                })
                .unwrap_or(true);
            if should_replace {
                best_match = Some((priority, normalized, path));
            }
        }
    }

    let (_, matched_archive_path, extracted_git) = best_match.ok_or_else(|| {
        OperationError::install(format!(
            "git executable not found in MinGit archive; expected one of: {}",
            MINGIT_GIT_ENTRY_SUFFIXES
                .iter()
                .map(|path| format!("`{}`", path.trim_start_matches('/')))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    Ok((extracted_git, matched_archive_path))
}

fn write_mingit_launcher(
    destination: &Path,
    managed_dir: &Path,
    extracted_git: &Path,
) -> OperationResult<()> {
    let relative_git = extracted_git.strip_prefix(managed_dir).map_err(|err| {
        OperationError::install(format!("git executable not under managed dir: {err}"))
    })?;
    let relative_git = relative_git.to_string_lossy().replace('/', "\\");
    let launcher = format!("@echo off\r\n\"%~dp0{relative_git}\" %*\r\n");
    write_file_atomically(
        launcher.as_bytes(),
        destination,
        &AtomicWriteOptions {
            overwrite_existing: true,
            create_parent_directories: true,
            require_non_empty: true,
            ..AtomicWriteOptions::default()
        },
    )
    .map_err(|err| OperationError::install(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{finalize_mingit_installation_with_launcher_writer, mingit_launcher_backup_path};
    use crate::error::OperationError;

    #[test]
    fn mingit_finalize_restores_previous_installation_when_launcher_write_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let portable_root = managed_dir.join("git-portable");
        let staging_root = managed_dir.join("git-portable.stage");
        let backup_root = managed_dir.join("git-portable.backup");
        let launcher_destination = managed_dir.join("git.cmd");
        std::fs::create_dir_all(portable_root.join("cmd")).expect("create old portable root");
        std::fs::create_dir_all(staging_root.join("cmd")).expect("create staging portable root");
        std::fs::write(portable_root.join("cmd").join("git.exe"), b"old").expect("write old git");
        std::fs::write(staging_root.join("cmd").join("git.exe"), b"new").expect("write staged git");
        std::fs::write(&launcher_destination, b"old-launcher").expect("write old launcher");

        let err = finalize_mingit_installation_with_launcher_writer(
            &portable_root,
            &staging_root,
            &backup_root,
            &launcher_destination,
            &mingit_launcher_backup_path(&launcher_destination),
            || Err(OperationError::install("launcher write failed")),
        )
        .expect_err("launcher failure should roll back portable root");

        assert!(err.to_string().contains("launcher write failed"));
        assert_eq!(
            std::fs::read(portable_root.join("cmd").join("git.exe")).expect("restored git"),
            b"old"
        );
        assert_eq!(
            std::fs::read(&launcher_destination).expect("restored launcher"),
            b"old-launcher"
        );
        assert!(
            !backup_root.exists(),
            "transaction cleanup should remove backup root"
        );
    }

    #[test]
    fn mingit_finalize_restores_previous_launcher_when_writer_updates_then_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let portable_root = managed_dir.join("git-portable");
        let staging_root = managed_dir.join("git-portable.stage");
        let backup_root = managed_dir.join("git-portable.backup");
        let launcher_destination = managed_dir.join("git.cmd");
        let launcher_backup = mingit_launcher_backup_path(&launcher_destination);
        std::fs::create_dir_all(portable_root.join("cmd")).expect("create old portable root");
        std::fs::create_dir_all(staging_root.join("cmd")).expect("create staging portable root");
        std::fs::write(portable_root.join("cmd").join("git.exe"), b"old").expect("write old git");
        std::fs::write(staging_root.join("cmd").join("git.exe"), b"new").expect("write staged git");
        std::fs::write(&launcher_destination, b"old-launcher").expect("write old launcher");

        let err = finalize_mingit_installation_with_launcher_writer(
            &portable_root,
            &staging_root,
            &backup_root,
            &launcher_destination,
            &launcher_backup,
            || {
                std::fs::write(&launcher_destination, b"new-launcher")
                    .map_err(|write_err| OperationError::install(write_err.to_string()))?;
                Err(OperationError::install("launcher post-write failure"))
            },
        )
        .expect_err("post-write failure should roll back both payload and launcher");

        assert!(err.to_string().contains("launcher post-write failure"));
        assert_eq!(
            std::fs::read(portable_root.join("cmd").join("git.exe")).expect("restored git"),
            b"old"
        );
        assert_eq!(
            std::fs::read(&launcher_destination).expect("restored launcher"),
            b"old-launcher"
        );
        assert!(!backup_root.exists(), "backup root should be cleaned");
        assert!(
            !launcher_backup.exists(),
            "launcher backup should be cleaned"
        );
    }

    #[test]
    fn mingit_finalize_restores_interrupted_backups_before_reinstall() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let portable_root = managed_dir.join("git-portable");
        let staging_root = managed_dir.join("git-portable.stage");
        let backup_root = managed_dir.join("git-portable.backup");
        let launcher_destination = managed_dir.join("git.cmd");
        let launcher_backup = mingit_launcher_backup_path(&launcher_destination);
        std::fs::create_dir_all(backup_root.join("cmd")).expect("create backup portable root");
        std::fs::create_dir_all(staging_root.join("cmd")).expect("create staging portable root");
        std::fs::write(backup_root.join("cmd").join("git.exe"), b"old").expect("write backup git");
        std::fs::write(staging_root.join("cmd").join("git.exe"), b"new").expect("write staged git");
        std::fs::write(&launcher_backup, b"old-launcher").expect("write backup launcher");

        finalize_mingit_installation_with_launcher_writer(
            &portable_root,
            &staging_root,
            &backup_root,
            &launcher_destination,
            &launcher_backup,
            || {
                std::fs::write(&launcher_destination, b"new-launcher")
                    .map_err(|err| OperationError::install(err.to_string()))
            },
        )
        .expect("interrupted backups should self-heal before reinstall");

        assert_eq!(
            std::fs::read(portable_root.join("cmd").join("git.exe")).expect("installed git"),
            b"new"
        );
        assert_eq!(
            std::fs::read(&launcher_destination).expect("installed launcher"),
            b"new-launcher"
        );
        assert!(!backup_root.exists(), "backup root should be cleaned");
        assert!(
            !launcher_backup.exists(),
            "launcher backup should be cleaned"
        );
    }
}
