use std::path::Path;

use omne_artifact_install_primitives::{
    ArtifactDownloadCandidate, ArtifactInstallError, BinaryArchiveInstallRequest,
    DownloadBinaryRequest, InstalledArchiveBinary, download_and_install_binary_from_archive,
    download_binary_to_destination, is_binary_archive_asset_name,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::download_sources::{
    build_download_candidates, redact_source_url, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::external_gateway::infer_gateway_candidate_for_git_release;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::plan_items::ReleasePlanItem;

use super::item_destination_resolution::{
    resolve_release_binary_name, resolve_release_destination,
};

pub(crate) async fn execute_release_item(
    item: &ReleasePlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let url = item.url.as_str().to_string();
    let binary_name = resolve_release_binary_name(item, target_triple);
    let destination = resolve_release_destination(item, target_triple, managed_dir);

    let gateway = infer_gateway_candidate_for_git_release(cfg, &url);
    let expected_sha = item.sha256.as_ref();

    let asset_name = item
        .url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.bin", item.id));
    let candidates = build_download_candidates(
        &url,
        &cfg.download_sources.mirror_prefixes,
        gateway.as_deref(),
    );
    if is_binary_archive_asset_name(&asset_name) {
        let archive_binary_hint =
            release_archive_binary_hint(&asset_name, item.archive_binary.as_deref());
        let fallback_archive_binary_hint = release_archive_binary_hint_fallback(
            item.archive_binary.as_deref(),
            archive_binary_hint.as_deref(),
        );
        let downloaded = download_release_archive_binary(
            client,
            &candidates,
            &BinaryArchiveInstallRequest {
                canonical_url: &url,
                destination: &destination,
                asset_name: &asset_name,
                binary_name: &binary_name,
                tool_name: &item.id,
                archive_binary_hint: archive_binary_hint.as_deref(),
                expected_sha256: expected_sha,
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
        return Ok(BootstrapItem {
            tool: item.id.clone(),
            status: BootstrapStatus::Installed,
            source: Some(redact_source_url(&source.url)),
            source_kind: Some(result_source_kind_for_download_candidate(source.kind)),
            archive_match: Some(archive_match.into()),
            destination: Some(destination.display().to_string()),
            detail: None,
            error_code: None,
            failure_code: None,
        });
    }

    let downloaded_source = download_binary_to_destination(
        client,
        &candidates,
        &DownloadBinaryRequest {
            canonical_url: &url,
            destination: &destination,
            asset_name: &asset_name,
            expected_sha256: expected_sha,
            max_download_bytes: cfg.download.max_download_bytes,
        },
    )
    .await
    .map_err(OperationError::from_artifact_install)?;

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(redact_source_url(&downloaded_source.url)),
        source_kind: Some(result_source_kind_for_download_candidate(
            downloaded_source.kind,
        )),
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

async fn download_release_archive_binary(
    client: &reqwest::Client,
    candidates: &[ArtifactDownloadCandidate],
    request: &BinaryArchiveInstallRequest<'_>,
    fallback_archive_binary_hint: Option<&str>,
) -> Result<InstalledArchiveBinary, ArtifactInstallError> {
    match download_and_install_binary_from_archive(client, candidates, request).await {
        Ok(downloaded) => Ok(downloaded),
        Err(err)
            if should_retry_release_archive_binary_with_fallback(
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

fn should_retry_release_archive_binary_with_fallback(
    err: &ArtifactInstallError,
    binary_name: &str,
    archive_binary_hint: Option<&str>,
    fallback_archive_binary_hint: Option<&str>,
) -> bool {
    archive_binary_hint != fallback_archive_binary_hint
        && fallback_archive_binary_hint.is_some()
        && err
            .to_string()
            .contains(&format!("binary `{binary_name}` not found"))
}

fn release_archive_binary_hint(asset_name: &str, archive_binary: Option<&str>) -> Option<String> {
    let normalized = normalize_archive_binary_hint(archive_binary)?;
    let Some(root) = archive_root_name(asset_name) else {
        return Some(normalized);
    };
    if normalized == root || normalized.starts_with(&format!("{root}/")) {
        return Some(normalized);
    }
    Some(format!("{root}/{normalized}"))
}

fn release_archive_binary_hint_fallback(
    archive_binary: Option<&str>,
    archive_binary_hint: Option<&str>,
) -> Option<String> {
    let normalized = normalize_archive_binary_hint(archive_binary)?;
    (Some(normalized.as_str()) != archive_binary_hint).then_some(normalized)
}

fn normalize_archive_binary_hint(archive_binary: Option<&str>) -> Option<String> {
    let hint = archive_binary?;
    let hint = hint.trim().replace('\\', "/");
    let hint = hint.trim_start_matches('/');
    (!hint.is_empty()).then_some(hint.to_string())
}

fn archive_root_name(asset_name: &str) -> Option<&str> {
    asset_name
        .strip_suffix(".tar.gz")
        .or_else(|| asset_name.strip_suffix(".tar.xz"))
        .or_else(|| asset_name.strip_suffix(".zip"))
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_archive_binary_hint, release_archive_binary_hint,
        release_archive_binary_hint_fallback,
    };

    #[test]
    fn release_archive_binary_hint_prefixes_archive_root_for_relative_hint() {
        assert_eq!(
            release_archive_binary_hint("node-v22.14.0-linux-x64.tar.xz", Some("bin/node")),
            Some("node-v22.14.0-linux-x64/bin/node".to_string())
        );
    }

    #[test]
    fn release_archive_binary_hint_keeps_exact_rooted_hint() {
        assert_eq!(
            release_archive_binary_hint(
                "node-v22.14.0-linux-x64.tar.xz",
                Some("node-v22.14.0-linux-x64/bin/node")
            ),
            Some("node-v22.14.0-linux-x64/bin/node".to_string())
        );
    }

    #[test]
    fn release_archive_binary_hint_fallback_keeps_original_unrooted_hint() {
        let primary = release_archive_binary_hint("7z2600-linux-x64.tar.xz", Some("7zz"));
        assert_eq!(
            release_archive_binary_hint_fallback(Some("7zz"), primary.as_deref()),
            Some("7zz".to_string())
        );
    }

    #[test]
    fn normalize_archive_binary_hint_normalizes_slashes_and_leading_root() {
        assert_eq!(
            normalize_archive_binary_hint(Some("\\demo\\bin\\demo.exe")),
            Some("demo/bin/demo.exe".to_string())
        );
    }
}
