use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use omne_fs_primitives::{AtomicWriteOptions, write_file_atomically};
use omne_integrity_primitives::parse_sha256_digest;
use omne_integrity_primitives::{Sha256Digest, verify_sha256_reader};
use zip::ZipArchive;

use crate::contracts::{BootstrapArchiveFormat, BootstrapArchiveMatch, InstallSource};
use crate::error::{OperationError, OperationResult};
use crate::installation::archive_binary::{
    InstalledArchiveDownload, download_and_install_binary_from_archive,
};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::source_acquisition::{
    DownloadOptions, GithubReleaseAsset, build_download_candidates,
    download_candidate_to_writer_with_options, fetch_latest_github_release,
    make_gateway_asset_candidate, result_source_kind_for_download_candidate,
};

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
    let release = fetch_latest_github_release(client, &cfg.github_api_bases, "cli/cli")
        .await
        .map_err(|err| OperationError::download(err.to_string()))?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(suffix))
        .ok_or_else(|| {
            OperationError::download(format!("cannot find gh release asset suffix `{suffix}`"))
        })?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref())
        .ok_or_else(|| OperationError::download("missing sha256 digest in gh release metadata"))?;
    let downloaded = download_and_install_binary_from_archive(
        client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
        None,
        destination,
        &asset.name,
        &format!("gh{binary_ext}"),
        "gh",
        Some(&format!("bin/gh{binary_ext}")),
        Some(&expected_sha),
        cfg.max_download_bytes,
    )
    .await?;
    let InstalledArchiveDownload {
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
    let release = fetch_latest_github_release(client, &cfg.github_api_bases, "git-for-windows/git")
        .await
        .map_err(|err| OperationError::download(err.to_string()))?;
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
    let gateway = if cfg.use_gateway_for_git_release() {
        cfg.gateway_base
            .as_deref()
            .map(|base| make_gateway_asset_candidate(base, "git", &release.tag_name, &asset.name))
    } else {
        None
    };
    if target_triple.contains("windows") {
        return download_and_install_mingit_bundle(
            client,
            &asset.browser_download_url,
            &cfg.mirror_prefixes,
            gateway.as_deref(),
            destination,
            &expected_sha,
            cfg.max_download_bytes,
        )
        .await;
    }

    let downloaded = download_and_install_binary_from_archive(
        client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
        gateway.as_deref(),
        destination,
        &asset.name,
        "git.exe",
        "git",
        None,
        Some(&expected_sha),
        cfg.max_download_bytes,
    )
    .await?;
    let InstalledArchiveDownload {
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
    assets: &'a [GithubReleaseAsset],
    target_triple: &str,
) -> Option<&'a GithubReleaseAsset> {
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

async fn download_and_install_mingit_bundle(
    client: &reqwest::Client,
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
    destination: &Path,
    expected_sha: &Sha256Digest,
    max_download_bytes: Option<u64>,
) -> OperationResult<InstallSource> {
    let candidates = build_download_candidates(canonical_url, mirror_prefixes, gateway_candidate);
    let mut errors = Vec::new();
    for candidate in candidates {
        let mut archive_bytes = Vec::new();
        let download_result = download_candidate_to_writer_with_options(
            client,
            &candidate,
            &mut archive_bytes,
            DownloadOptions {
                max_bytes: max_download_bytes,
            },
        )
        .await;
        if let Err(err) = download_result {
            errors.push(format!(
                "{}:{} -> {err}",
                candidate.kind.label(),
                candidate.url
            ));
            continue;
        }

        let mut reader = Cursor::new(&archive_bytes);
        verify_sha256_reader(&mut reader, expected_sha)
            .map_err(|err| OperationError::download(err.to_string()))?;
        let archive_match = install_mingit_bundle_from_zip_bytes(&archive_bytes, destination)?;
        return Ok(InstallSource::new(
            candidate.url,
            result_source_kind_for_download_candidate(candidate.kind),
        )
        .with_archive_match(archive_match));
    }

    Err(OperationError::download(format!(
        "all download candidates failed for {canonical_url}: {}",
        errors.join(" | ")
    )))
}

fn install_mingit_bundle_from_zip_bytes(
    archive_bytes: &[u8],
    destination: &Path,
) -> OperationResult<BootstrapArchiveMatch> {
    let managed_dir = destination.parent().ok_or_else(|| {
        OperationError::install(format!(
            "cannot determine managed dir for {}",
            destination.display()
        ))
    })?;
    let portable_root = managed_dir.join("git-portable");
    if portable_root.exists() {
        fs::remove_dir_all(&portable_root)
            .map_err(|err| OperationError::install(err.to_string()))?;
    }
    fs::create_dir_all(&portable_root).map_err(|err| OperationError::install(err.to_string()))?;

    let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|err| OperationError::install(err.to_string()))?;
    let mut extracted_git: Option<(usize, PathBuf)> = None;
    let mut matched_archive_path: Option<(usize, String)> = None;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| OperationError::install(err.to_string()))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| {
                OperationError::install(format!("unsafe archive entry path at index {index}"))
            })?
            .to_path_buf();
        let output_path = portable_root.join(&enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|err| OperationError::install(err.to_string()))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| OperationError::install(err.to_string()))?;
        }
        let mut file =
            File::create(&output_path).map_err(|err| OperationError::install(err.to_string()))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|err| OperationError::install(err.to_string()))?;

        let normalized = enclosed.to_string_lossy().replace('\\', "/");
        if let Some(priority) = mingit_git_entry_priority(&normalized) {
            let should_replace = extracted_git
                .as_ref()
                .map(|(current_priority, _)| priority < *current_priority)
                .unwrap_or(true);
            if should_replace {
                extracted_git = Some((priority, output_path));
                matched_archive_path = Some((priority, normalized));
            }
        }
    }

    let (_, extracted_git) = extracted_git.ok_or_else(|| {
        OperationError::install(format!(
            "git executable not found in MinGit archive; expected one of: {}",
            MINGIT_GIT_ENTRY_SUFFIXES
                .iter()
                .map(|path| format!("`{}`", path.trim_start_matches('/')))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let (_, matched_archive_path) =
        matched_archive_path.expect("matched path set with extracted git");
    write_mingit_launcher(destination, managed_dir, &extracted_git)?;

    Ok(BootstrapArchiveMatch {
        format: BootstrapArchiveFormat::Zip,
        path: matched_archive_path,
    })
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
