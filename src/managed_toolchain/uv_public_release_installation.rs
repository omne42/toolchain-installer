use std::path::Path;

use github_kit::GitHubReleaseAsset;
use omne_artifact_install_primitives::{
    ArtifactInstallErrorKind, BinaryArchiveInstallRequest, InstalledArchiveBinary,
    download_and_install_binary_from_archive,
};
use omne_integrity_primitives::parse_sha256_digest;

use crate::artifact::InstallSource;
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::github_release_metadata::{
    build_github_release_http_client, fetch_latest_release_metadata,
};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::managed_environment_layout::validated_binary_suffix;
pub(crate) async fn install_uv_from_public_release(
    target_triple: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    _client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    let github_client = build_github_release_http_client(cfg)?;
    let release = fetch_latest_release_metadata(&github_client, cfg, "astral-sh/uv").await?;
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
    let binary_name = format!("uv{}", validated_binary_suffix(target_triple));
    let archive_binary_hint = uv_archive_binary_hint(&asset.name, &binary_name);
    let primary_request = BinaryArchiveInstallRequest {
        canonical_url: &asset.browser_download_url,
        destination,
        asset_name: &asset.name,
        binary_name: &binary_name,
        archive_binary_hint: archive_binary_hint.as_deref(),
        expected_sha256: Some(&expected_sha),
        max_download_bytes: cfg.download.max_download_bytes,
    };
    let downloaded = match download_and_install_binary_from_archive(
        &github_client,
        &candidates,
        &primary_request,
    )
    .await
    {
        Ok(downloaded) => downloaded,
        Err(err)
            if err.kind() == ArtifactInstallErrorKind::Install
                && err.to_string().contains(" not found in ")
                && archive_binary_hint.as_deref() != Some(binary_name.as_str()) =>
        {
            let fallback_request = BinaryArchiveInstallRequest {
                archive_binary_hint: Some(binary_name.as_str()),
                ..primary_request
            };
            download_and_install_binary_from_archive(&github_client, &candidates, &fallback_request)
                .await
                .map_err(OperationError::from_artifact_install)?
        }
        Err(err) => return Err(OperationError::from_artifact_install(err)),
    };
    let InstalledArchiveBinary {
        source,
        archive_match,
    } = downloaded;
    Ok(InstallSource::new(
        source.url,
        result_source_kind_for_download_candidate(&source.source_label),
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

fn uv_archive_binary_hint(asset_name: &str, binary_name: &str) -> Option<String> {
    let root = archive_root_name(asset_name)?;
    Some(format!("{root}/{binary_name}"))
}

fn archive_root_name(asset_name: &str) -> Option<&str> {
    asset_name
        .strip_suffix(".tar.gz")
        .or_else(|| asset_name.strip_suffix(".tar.xz"))
        .or_else(|| asset_name.strip_suffix(".zip"))
}
