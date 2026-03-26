use std::path::Path;

use github_kit::{
    DEFAULT_GITHUB_API_VERSION, GITHUB_API_ACCEPT, GitHubRelease, GitHubReleaseAsset,
};
use http_kit::{
    DEFAULT_MAX_RESPONSE_BODY_BYTES, http_status_text_error, read_json_body_limited,
    read_text_body_limited, redact_url_for_error, redact_url_str, response_body_read_error,
    send_reqwest,
};
use omne_artifact_install_primitives::{
    BinaryArchiveInstallRequest, InstalledArchiveBinary, download_and_install_binary_from_archive,
};
use omne_host_info_primitives::executable_suffix_for_target;
use omne_integrity_primitives::parse_sha256_digest;
use reqwest::header::{ACCEPT, USER_AGENT};

use crate::artifact::InstallSource;
use crate::download_sources::{
    build_download_candidates, result_source_kind_for_download_candidate,
};
use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;

const GITHUB_RELEASE_METADATA_MAX_BYTES: usize = 256 * 1024;

pub(crate) async fn install_uv_from_public_release(
    target_triple: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    let release = fetch_latest_release_metadata(
        client,
        &cfg.github_releases.api_bases,
        cfg.github_releases.token.as_deref(),
        "astral-sh/uv",
    )
    .await?;
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

async fn fetch_latest_release_metadata(
    client: &reqwest::Client,
    api_bases: &[String],
    bearer_token: Option<&str>,
    repo: &str,
) -> OperationResult<GitHubRelease> {
    let Some((owner, name)) = repo.trim().split_once('/') else {
        return Err(OperationError::download(format!(
            "github repository must be `owner/repo`, got `{repo}`"
        )));
    };
    if owner.is_empty()
        || name.is_empty()
        || name.contains('/')
        || owner.chars().any(char::is_whitespace)
        || name.chars().any(char::is_whitespace)
    {
        return Err(OperationError::download(format!(
            "github repository must be `owner/repo`, got `{repo}`"
        )));
    }

    let mut attempted = false;
    let mut errors = Vec::new();

    for base in api_bases {
        let trimmed = base.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        attempted = true;

        let url = match build_latest_release_url(trimmed, owner, name) {
            Ok(url) => url,
            Err(err) => {
                errors.push(format!("{} -> {err}", redact_url_str(trimmed)));
                continue;
            }
        };
        let redacted_url = redact_url_for_error(&url);

        let mut request = client
            .get(url.clone())
            .header(ACCEPT, GITHUB_API_ACCEPT)
            .header(USER_AGENT, "toolchain-installer")
            .header("X-GitHub-Api-Version", DEFAULT_GITHUB_API_VERSION);
        if let Some(token) = bearer_token
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            request = request.bearer_auth(token);
        }

        let response = match send_reqwest(request, "github latest release").await {
            Ok(response) => response,
            Err(err) => {
                errors.push(format!("{redacted_url} -> {err}"));
                continue;
            }
        };

        let status = response.status();
        let json = if status.is_success() {
            match read_json_body_limited(response, GITHUB_RELEASE_METADATA_MAX_BYTES).await {
                Ok(json) => json,
                Err(err) => {
                    errors.push(format!("{redacted_url} -> {err}"));
                    continue;
                }
            }
        } else {
            let err = match read_text_body_limited(response, DEFAULT_MAX_RESPONSE_BODY_BYTES).await
            {
                Ok(body) => http_status_text_error("github latest release", status, &body),
                Err(err) => {
                    response_body_read_error("github latest release http error", status, &err)
                }
            };
            errors.push(format!("{redacted_url} -> {err}"));
            continue;
        };

        match serde_json::from_value::<GitHubRelease>(json) {
            Ok(release) => return Ok(release),
            Err(err) => errors.push(format!("{redacted_url} -> invalid json: {err}")),
        }
    }

    if !attempted {
        return Err(OperationError::download(
            "no usable github api base configured".to_string(),
        ));
    }

    Err(OperationError::download(format!(
        "failed to fetch latest release metadata for {repo}: {}",
        errors.join(" | ")
    )))
}

fn build_latest_release_url(base: &str, owner: &str, name: &str) -> Result<reqwest::Url, String> {
    let mut url =
        reqwest::Url::parse(base).map_err(|err| format!("invalid github api base: {err}"))?;
    url.path_segments_mut()
        .map_err(|_| "invalid github api base".to_string())?
        .extend(["repos", owner, name, "releases", "latest"]);
    Ok(url)
}
