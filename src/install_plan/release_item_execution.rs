use std::path::Path;

use omne_artifact_install_primitives::{
    BinaryArchiveInstallRequest, DownloadBinaryRequest, download_and_install_binary_from_archive,
    download_binary_to_destination, is_binary_archive_asset_name,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
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
        let archive_binary_hint = normalize_archive_binary_hint(item.archive_binary.as_deref());
        let downloaded = download_and_install_binary_from_archive(
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
        )
        .await
        .map_err(OperationError::from_artifact_install)?;
        let source = downloaded.source;
        let archive_match = downloaded.archive_match;
        return Ok(BootstrapItem {
            tool: item.id.clone(),
            status: BootstrapStatus::Installed,
            source: Some(source.url),
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
        source: Some(downloaded_source.url),
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

fn normalize_archive_binary_hint(archive_binary: Option<&str>) -> Option<String> {
    let hint = archive_binary?;
    let hint = hint.trim().replace('\\', "/");
    let hint = hint.trim_start_matches('/');
    (!hint.is_empty()).then_some(hint.to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_archive_binary_hint;

    #[test]
    fn normalize_archive_binary_hint_normalizes_slashes_and_leading_root() {
        assert_eq!(
            normalize_archive_binary_hint(Some("\\demo\\bin\\demo.exe")),
            Some("demo/bin/demo.exe".to_string())
        );
    }
}
