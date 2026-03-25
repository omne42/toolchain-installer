use std::path::Path;

use omne_integrity_primitives::parse_sha256_digest;

use crate::contracts::InstallSource;
use crate::error::{OperationError, OperationResult};
use crate::installation::archive_binary::{
    InstalledArchiveDownload, download_and_install_binary_from_archive,
};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::source_acquisition::{
    GithubReleaseAsset, fetch_latest_github_release, make_gateway_asset_candidate,
    result_source_kind_for_download_candidate,
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
