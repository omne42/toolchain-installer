use std::fs;
use std::path::{Path, PathBuf};

use github_kit::{
    DEFAULT_GITHUB_API_VERSION, GITHUB_API_ACCEPT, GitHubRelease, GitHubReleaseAsset,
};
use http_kit::{
    DEFAULT_MAX_RESPONSE_BODY_BYTES, http_status_text_error, read_json_body_limited,
    read_text_body_limited, redact_url_for_error, redact_url_str, response_body_read_error,
    send_reqwest,
};
use omne_artifact_install_primitives::{
    ArchiveTreeInstallRequest, BinaryArchiveInstallRequest, InstalledArchiveBinary,
    download_and_install_archive_tree, download_and_install_binary_from_archive,
};
use omne_fs_primitives::{AtomicWriteOptions, write_file_atomically};
use omne_integrity_primitives::{Sha256Digest, parse_sha256_digest};
use reqwest::header::{ACCEPT, USER_AGENT};

use crate::artifact::InstallSource;
use crate::contracts::{BootstrapArchiveFormat, BootstrapArchiveMatch};
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::external_gateway::gateway_candidate_for_git_release_asset;
use crate::installer_runtime_config::InstallerRuntimeConfig;

const GITHUB_RELEASE_METADATA_MAX_BYTES: usize = 256 * 1024;

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
    let release = fetch_latest_release_metadata(
        client,
        &cfg.github_releases.api_bases,
        cfg.github_releases.token.as_deref(),
        "cli/cli",
    )
    .await?;
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
    let release = fetch_latest_release_metadata(
        client,
        &cfg.github_releases.api_bases,
        cfg.github_releases.token.as_deref(),
        "git-for-windows/git",
    )
    .await?;
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

async fn fetch_latest_release_metadata(
    client: &reqwest::Client,
    api_bases: &[String],
    bearer_token: Option<&str>,
    repo: &str,
) -> OperationResult<GitHubRelease> {
    let Some((owner, name)) = repo.trim().split_once('/') else {
        return Err(OperationError::download(format!(
            "github repository must be `owner/repo`, got `{repo}`"
        )));
    };
    if owner.is_empty()
        || name.is_empty()
        || name.contains('/')
        || owner.chars().any(char::is_whitespace)
        || name.chars().any(char::is_whitespace)
    {
        return Err(OperationError::download(format!(
            "github repository must be `owner/repo`, got `{repo}`"
        )));
    }

    let mut attempted = false;
    let mut errors = Vec::new();

    for base in api_bases {
        let trimmed = base.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        attempted = true;

        let url = match build_latest_release_url(trimmed, owner, name) {
            Ok(url) => url,
            Err(err) => {
                errors.push(format!("{} -> {err}", redact_url_str(trimmed)));
                continue;
            }
        };
        let redacted_url = redact_url_for_error(&url);

        let mut request = client
            .get(url.clone())
            .header(ACCEPT, GITHUB_API_ACCEPT)
            .header(USER_AGENT, "toolchain-installer")
            .header("X-GitHub-Api-Version", DEFAULT_GITHUB_API_VERSION);
        if let Some(token) = bearer_token
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            request = request.bearer_auth(token);
        }

        let response = match send_reqwest(request, "github latest release").await {
            Ok(response) => response,
            Err(err) => {
                errors.push(format!("{redacted_url} -> {err}"));
                continue;
            }
        };

        let status = response.status();
        let json = if status.is_success() {
            match read_json_body_limited(response, GITHUB_RELEASE_METADATA_MAX_BYTES).await {
                Ok(json) => json,
                Err(err) => {
                    errors.push(format!("{redacted_url} -> {err}"));
                    continue;
                }
            }
        } else {
            let err = match read_text_body_limited(response, DEFAULT_MAX_RESPONSE_BODY_BYTES).await
            {
                Ok(body) => http_status_text_error("github latest release", status, &body),
                Err(err) => {
                    response_body_read_error("github latest release http error", status, &err)
                }
            };
            errors.push(format!("{redacted_url} -> {err}"));
            continue;
        };

        match serde_json::from_value::<GitHubRelease>(json) {
            Ok(release) => return Ok(release),
            Err(err) => errors.push(format!("{redacted_url} -> invalid json: {err}")),
        }
    }

    if !attempted {
        return Err(OperationError::download(
            "no usable github api base configured".to_string(),
        ));
    }

    Err(OperationError::download(format!(
        "failed to fetch latest release metadata for {repo}: {}",
        errors.join(" | ")
    )))
}

fn build_latest_release_url(base: &str, owner: &str, name: &str) -> Result<reqwest::Url, String> {
    let mut url =
        reqwest::Url::parse(base).map_err(|err| format!("invalid github api base: {err}"))?;
    url.path_segments_mut()
        .map_err(|_| "invalid github api base".to_string())?
        .extend(["repos", owner, name, "releases", "latest"]);
    Ok(url)
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
    let final_git = portable_root.join(relative_git);
    replace_mingit_installation(&portable_root, &staging_root, &backup_root)?;
    write_mingit_launcher(destination, managed_dir, &final_git)?;

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

pub(crate) fn replace_mingit_installation(
    portable_root: &Path,
    staging_root: &Path,
    backup_root: &Path,
) -> OperationResult<()> {
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

    let _ = remove_dir_if_exists(backup_root);
    Ok(())
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
