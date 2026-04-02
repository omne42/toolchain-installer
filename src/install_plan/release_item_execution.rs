use std::path::Path;

use omne_artifact_install_primitives::{
    BinaryArchiveInstallRequest, DownloadBinaryRequest, InstalledArchiveBinary,
    download_and_install_binary_from_archive, download_binary_to_destination,
    is_binary_archive_asset_name,
};
use reqwest::Url;

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::download_sources::{
    build_download_candidates, redact_source_url, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::external_gateway::gateway_candidate_for_git_release_download_url;
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

    let gateway = gateway_candidate_for_git_release_download_url(cfg, &url);
    let expected_sha = item.sha256.as_ref();
    let github_client = build_release_download_client(cfg, &url)?;
    let download_client = github_client.as_ref().unwrap_or(client);

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
        let downloaded = download_and_install_binary_from_archive(
            download_client,
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
        download_client,
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

fn build_release_download_client(
    cfg: &InstallerRuntimeConfig,
    url: &str,
) -> OperationResult<Option<reqwest::Client>> {
    if !is_github_release_asset_url(url) {
        return Ok(None);
    }
    reqwest::Client::builder()
        .http1_only()
        .timeout(cfg.download.http_timeout)
        .user_agent("toolchain-installer")
        .build()
        .map(Some)
        .map_err(|err| OperationError::download(format!("build github http client failed: {err}")))
}

fn is_github_release_asset_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.host_str() != Some("github.com") {
        return false;
    }
    let Some(segments) = parsed.path_segments() else {
        return false;
    };
    let segments = segments.collect::<Vec<_>>();
    segments.len() >= 6 && segments[2] == "releases" && segments[3] == "download"
}

#[cfg(test)]
mod tests {
    use super::{
        is_github_release_asset_url, normalize_archive_binary_hint, release_archive_binary_hint,
    };

    #[test]
    fn normalize_archive_binary_hint_normalizes_slashes_and_leading_root() {
        assert_eq!(
            normalize_archive_binary_hint(Some("\\demo\\bin\\demo.exe")),
            Some("demo/bin/demo.exe".to_string())
        );
    }

    #[test]
    fn normalize_archive_binary_hint_keeps_relative_and_rooted_values_verbatim() {
        assert_eq!(
            normalize_archive_binary_hint(Some("bin/node")),
            Some("bin/node".to_string())
        );
        assert_eq!(
            normalize_archive_binary_hint(Some("node-v22.14.0-linux-x64/bin/node")),
            Some("node-v22.14.0-linux-x64/bin/node".to_string())
        );
    }

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
    fn github_release_asset_url_detection_matches_release_download_shape() {
        assert!(is_github_release_asset_url(
            "https://github.com/cli/cli/releases/download/v2.0.0/gh_2.0.0_linux_amd64.tar.gz"
        ));
        assert!(!is_github_release_asset_url(
            "https://mirror.example/github.com/cli/cli/releases/download/v2.0.0/gh.tar.gz"
        ));
        assert!(!is_github_release_asset_url(
            "https://github.com/cli/cli/archive/refs/tags/v2.0.0.tar.gz"
        ));
    }
}
