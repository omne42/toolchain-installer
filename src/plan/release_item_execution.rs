use std::io::{Seek, SeekFrom};
use std::path::Path;

use omne_fs_primitives::{AtomicWriteOptions, stage_file_atomically_with_name};
use omne_host_info_primitives::executable_suffix_for_target;
use omne_integrity_primitives::{Sha256Digest, parse_sha256_user_input, verify_sha256_reader};

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::installation::archive_binary::{
    InstalledArchiveDownload, download_and_install_binary_from_archive,
    is_supported_archive_asset_name,
};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::source_acquisition::{
    DownloadCandidate, DownloadOptions, build_download_candidates,
    download_candidate_to_writer_with_options, infer_gateway_candidate_for_git_release,
    result_source_kind_for_download_candidate,
};

use super::item_destination_resolution::resolve_release_destination;

pub(crate) async fn execute_release_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<BootstrapItem> {
    let url = item
        .url
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("release method requires `url`"))?;
    let binary_name = item
        .binary_name
        .clone()
        .unwrap_or_else(|| format!("{}{}", item.id, executable_suffix_for_target(target_triple)));
    let destination = resolve_release_destination(item, target_triple, managed_dir);

    let gateway = if cfg.use_gateway_for_git_release() && item.id == "git" {
        infer_gateway_candidate_for_git_release(cfg, &url)
    } else {
        None
    };
    let expected_sha = item
        .sha256
        .as_deref()
        .map(|raw_sha| {
            parse_sha256_user_input(raw_sha).ok_or_else(|| {
                OperationError::install(format!("invalid sha256 value for `{}`", item.id))
            })
        })
        .transpose()?;

    let asset_name = url
        .rsplit('/')
        .next()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.bin", item.id));
    if is_supported_archive_asset_name(&asset_name) {
        let downloaded = download_and_install_binary_from_archive(
            client,
            &url,
            &cfg.mirror_prefixes,
            gateway.as_deref(),
            &destination,
            &asset_name,
            &binary_name,
            &item.id,
            item.archive_binary.as_deref(),
            expected_sha.as_ref(),
            cfg.max_download_bytes,
        )
        .await?;
        let InstalledArchiveDownload {
            source,
            archive_match,
        } = downloaded;
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

    let downloaded_source = download_release_binary_to_destination(
        client,
        &url,
        &cfg.mirror_prefixes,
        gateway.as_deref(),
        &destination,
        &asset_name,
        expected_sha.as_ref(),
        cfg.max_download_bytes,
    )
    .await?;

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

#[allow(clippy::too_many_arguments)]
async fn download_release_binary_to_destination(
    client: &reqwest::Client,
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
    destination: &Path,
    asset_name: &str,
    expected_sha: Option<&Sha256Digest>,
    max_download_bytes: Option<u64>,
) -> OperationResult<DownloadCandidate> {
    let candidates = build_download_candidates(canonical_url, mirror_prefixes, gateway_candidate);
    let mut errors = Vec::new();
    for candidate in candidates {
        let mut staged = stage_file_atomically_with_name(
            destination,
            &release_binary_write_options(),
            Some(asset_name),
        )
        .map_err(|err| OperationError::install(err.to_string()))?;
        let download_result = download_candidate_to_writer_with_options(
            client,
            &candidate,
            staged.file_mut(),
            DownloadOptions {
                max_bytes: max_download_bytes,
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

        if let Some(expected_sha) = expected_sha {
            staged
                .file_mut()
                .seek(SeekFrom::Start(0))
                .map_err(|err| OperationError::install(err.to_string()))?;
            verify_sha256_reader(staged.file_mut(), expected_sha)
                .map_err(|err| OperationError::download(err.to_string()))?;
        }

        staged
            .commit()
            .map_err(|err| OperationError::install(err.to_string()))?;
        return Ok(candidate);
    }
    Err(OperationError::download(format!(
        "all download candidates failed for {canonical_url}: {}",
        errors.join(" | ")
    )))
}

fn release_binary_write_options() -> AtomicWriteOptions {
    AtomicWriteOptions {
        overwrite_existing: true,
        create_parent_directories: true,
        require_non_empty: true,
        require_executable_on_unix: true,
        unix_mode: Some(0o755),
    }
}
