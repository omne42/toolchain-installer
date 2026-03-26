use std::path::Path;

use github_kit::{GitHubReleaseAsset, fetch_latest_release};
use omne_artifact_install_primitives::{
    BinaryArchiveInstallRequest, InstalledArchiveBinary, download_and_install_binary_from_archive,
};
use omne_host_info_primitives::executable_suffix_for_target;
use omne_integrity_primitives::parse_sha256_digest;

use crate::artifact::InstallSource;
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;

pub(crate) async fn install_uv_from_public_release(
    target_triple: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    let request_options = cfg.github_releases.api_request_options();
    let release = fetch_latest_release(
        client,
        &cfg.github_releases.api_bases,
        "astral-sh/uv",
        request_options,
    )
    .await
    .map_err(|err| OperationError::download(err.to_string()))?;
    let asset = select_uv_asset_for_target(&release.assets, target_triple).ok_or_else(|| {
        OperationError::download(format!("cannot find uv asset for target `{target_triple}`"))
    })?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref())
        .ok_or_else(|| OperationError::download("missing sha256 digest in uv release metadata"))?;
    let candidates = build_download_candidates(
        &asset.browser_download_url,
        &cfg.download_sources.mirror_prefixes,
        None,
    );
    let downloaded = download_and_install_binary_from_archive(
        client,
        &candidates,
        &BinaryArchiveInstallRequest {
            canonical_url: &asset.browser_download_url,
            destination,
            asset_name: &asset.name,
            binary_name: &format!("uv{}", executable_suffix_for_target(target_triple)),
            tool_name: "uv",
            archive_binary_hint: Some(&format!(
                "uv{}",
                executable_suffix_for_target(target_triple)
            )),
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

fn select_uv_asset_for_target<'a>(
    assets: &'a [GitHubReleaseAsset],
    target_triple: &str,
) -> Option<&'a GitHubReleaseAsset> {
    let archive_ext = if target_triple.contains("windows") {
        ".zip"
    } else {
        ".tar.gz"
    };
    let name = format!("uv-{target_triple}{archive_ext}");
    assets.iter().find(|asset| asset.name == name)
}
