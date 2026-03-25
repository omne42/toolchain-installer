use std::io::Cursor;
use std::path::Path;

use omne_integrity_primitives::{Sha256Digest, parse_sha256_user_input, verify_sha256_reader};

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::installation::archive_tree::{
    extract_archive_tree_from_bytes, is_supported_tree_archive_asset_name,
};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::source_acquisition::{
    DownloadOptions, build_download_candidates, download_candidate_to_writer_with_options,
    infer_gateway_candidate_for_git_release, result_source_kind_for_download_candidate,
};

use super::item_destination_resolution::resolve_archive_tree_destination;

pub(crate) async fn execute_archive_tree_release_item(
    item: &InstallPlanItem,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let url = item
        .url
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("archive_tree_release method requires `url`"))?;
    let destination = resolve_archive_tree_destination(item, managed_dir);
    let asset_name = url
        .rsplit('/')
        .next()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.archive", item.id));
    if !is_supported_tree_archive_asset_name(&asset_name) {
        return Err(OperationError::install(format!(
            "archive_tree_release requires a supported archive asset, got `{asset_name}`"
        )));
    }
    let expected_sha = item
        .sha256
        .as_deref()
        .map(|raw_sha| {
            parse_sha256_user_input(raw_sha).ok_or_else(|| {
                OperationError::install(format!("invalid sha256 value for `{}`", item.id))
            })
        })
        .transpose()?;
    let gateway = if cfg.use_gateway_for_git_release() && item.id == "git" {
        infer_gateway_candidate_for_git_release(cfg, &url)
    } else {
        None
    };

    let candidates = build_download_candidates(&url, &cfg.mirror_prefixes, gateway.as_deref());
    let mut errors = Vec::new();
    for candidate in candidates {
        let mut archive_bytes = Vec::new();
        let download_result = download_candidate_to_writer_with_options(
            client,
            &candidate,
            &mut archive_bytes,
            DownloadOptions {
                max_bytes: cfg.max_download_bytes,
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

        verify_optional_sha(expected_sha.as_ref(), &archive_bytes)?;
        extract_archive_tree_from_bytes(&asset_name, &archive_bytes, &destination)?;
        return Ok(BootstrapItem {
            tool: item.id.clone(),
            status: BootstrapStatus::Installed,
            source: Some(candidate.url),
            source_kind: Some(result_source_kind_for_download_candidate(candidate.kind)),
            archive_match: None,
            destination: Some(destination.display().to_string()),
            detail: None,
            error_code: None,
            failure_code: None,
        });
    }

    Err(OperationError::download(format!(
        "all download candidates failed for {url}: {}",
        errors.join(" | ")
    )))
}

fn verify_optional_sha(
    expected_sha: Option<&Sha256Digest>,
    archive_bytes: &[u8],
) -> OperationResult<()> {
    if let Some(expected_sha) = expected_sha {
        let mut reader = Cursor::new(archive_bytes);
        verify_sha256_reader(&mut reader, expected_sha)
            .map_err(|err| OperationError::download(err.to_string()))?;
    }
    Ok(())
}
