use std::path::Path;

use omne_host_info_primitives::executable_suffix_for_target;
use omne_integrity_primitives::parse_sha256_digest;

use crate::contracts::InstallSource;
use crate::error::{OperationError, OperationResult};
use crate::installation::archive_binary::{
    InstalledArchiveDownload, download_and_install_binary_from_archive,
};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::source_acquisition::{
    GithubReleaseAsset, fetch_latest_github_release, result_source_kind_for_download_candidate,
};

pub(crate) async fn install_uv_from_public(
    target_triple: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    let release = fetch_latest_github_release(client, &cfg.github_api_bases, "astral-sh/uv")
        .await
        .map_err(|err| OperationError::download(err.to_string()))?;
    let asset = select_uv_asset_for_target(&release.assets, target_triple).ok_or_else(|| {
        OperationError::download(format!("cannot find uv asset for target `{target_triple}`"))
    })?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref())
        .ok_or_else(|| OperationError::download("missing sha256 digest in uv release metadata"))?;
    let downloaded = download_and_install_binary_from_archive(
        client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
        None,
        destination,
        &asset.name,
        &format!("uv{}", executable_suffix_for_target(target_triple)),
        "uv",
        Some(&format!(
            "uv{}",
            executable_suffix_for_target(target_triple)
        )),
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

pub(crate) fn select_uv_asset_for_target<'a>(
    assets: &'a [GithubReleaseAsset],
    target_triple: &str,
) -> Option<&'a GithubReleaseAsset> {
    let archive_ext = if target_triple.contains("windows") {
        ".zip"
    } else {
        ".tar.gz"
    };
    let name = format!("uv-{target_triple}{archive_ext}");
    assets.iter().find(|asset| asset.name == name)
}
