use std::path::Path;

use github_kit::{GitHubApiRequestOptions, GitHubReleaseAsset, fetch_latest_release};
use omne_artifact_install_primitives::{
    ArtifactDownloadCandidate, ArtifactInstallError, ArtifactInstallErrorKind,
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
    let release = fetch_latest_release(
        client,
        &cfg.github_releases.api_bases,
        "astral-sh/uv",
        GitHubApiRequestOptions::new()
            .with_user_agent("toolchain-installer")
            .with_bearer_token(cfg.github_releases.token.as_deref()),
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
    let binary_name = format!("uv{}", executable_suffix_for_target(target_triple));
    let archive_binary_hint = uv_archive_binary_hint(&asset.name, &binary_name);
    let fallback_archive_binary_hint =
        uv_archive_binary_hint_fallback(&binary_name, archive_binary_hint.as_deref());
    let downloaded = download_uv_archive_binary(
        client,
        &candidates,
        &BinaryArchiveInstallRequest {
            canonical_url: &asset.browser_download_url,
            destination,
            asset_name: &asset.name,
            binary_name: &binary_name,
            tool_name: "uv",
            archive_binary_hint: archive_binary_hint.as_deref(),
            expected_sha256: Some(&expected_sha),
            max_download_bytes: cfg.download.max_download_bytes,
        },
        fallback_archive_binary_hint.as_deref(),
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

async fn download_uv_archive_binary(
    client: &reqwest::Client,
    candidates: &[ArtifactDownloadCandidate],
    request: &BinaryArchiveInstallRequest<'_>,
    fallback_archive_binary_hint: Option<&str>,
) -> Result<InstalledArchiveBinary, ArtifactInstallError> {
    match download_and_install_binary_from_archive(client, candidates, request).await {
        Ok(downloaded) => Ok(downloaded),
        Err(err)
            if should_retry_uv_archive_with_fallback(
                &err,
                request.binary_name,
                request.archive_binary_hint,
                fallback_archive_binary_hint,
            ) =>
        {
            let fallback_request = BinaryArchiveInstallRequest {
                archive_binary_hint: fallback_archive_binary_hint,
                ..*request
            };
            download_and_install_binary_from_archive(client, candidates, &fallback_request).await
        }
        Err(err) => Err(err),
    }
}

fn should_retry_uv_archive_with_fallback(
    err: &ArtifactInstallError,
    binary_name: &str,
    archive_binary_hint: Option<&str>,
    fallback_archive_binary_hint: Option<&str>,
) -> bool {
    err.kind() == ArtifactInstallErrorKind::Install
        && archive_binary_hint != fallback_archive_binary_hint
        && fallback_archive_binary_hint.is_some()
        && err
            .to_string()
            .contains(&format!("binary `{binary_name}` not found"))
}

fn uv_archive_binary_hint(asset_name: &str, binary_name: &str) -> Option<String> {
    let root = archive_root_name(asset_name)?;
    Some(format!("{root}/{binary_name}"))
}

fn uv_archive_binary_hint_fallback(
    binary_name: &str,
    archive_binary_hint: Option<&str>,
) -> Option<String> {
    (Some(binary_name) != archive_binary_hint).then(|| binary_name.to_string())
}

fn archive_root_name(asset_name: &str) -> Option<&str> {
    asset_name
        .strip_suffix(".tar.gz")
        .or_else(|| asset_name.strip_suffix(".tar.xz"))
        .or_else(|| asset_name.strip_suffix(".zip"))
}
