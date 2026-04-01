use std::path::Path;

use omne_artifact_install_primitives::{
    ArchiveTreeInstallRequest, download_and_install_archive_tree, is_archive_tree_asset_name,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::external_gateway::infer_gateway_candidate_for_git_release;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::plan_items::ArchiveTreeReleasePlanItem;

pub(crate) async fn execute_archive_tree_release_item(
    item: &ArchiveTreeReleasePlanItem,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let url = item.url.as_str().to_string();
    let destination = item
        .destination
        .as_deref()
        .map(|destination| {
            super::item_destination_resolution::resolve_destination_path(destination, managed_dir)
        })
        .unwrap_or_else(|| managed_dir.join(&item.id));
    let asset_name = item
        .url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.archive", item.id));
    if !is_archive_tree_asset_name(&asset_name) {
        return Err(OperationError::install(format!(
            "archive_tree_release requires a supported archive asset, got `{asset_name}`"
        )));
    }
    let expected_sha = item.sha256.as_ref();
    let gateway = infer_gateway_candidate_for_git_release(cfg, &url);

    let candidates = build_download_candidates(
        &url,
        &cfg.download_sources.mirror_prefixes,
        gateway.as_deref(),
    );
    let selected = download_and_install_archive_tree(
        client,
        &candidates,
        &ArchiveTreeInstallRequest {
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
        source: Some(selected.url),
        source_kind: Some(result_source_kind_for_download_candidate(selected.kind)),
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}
