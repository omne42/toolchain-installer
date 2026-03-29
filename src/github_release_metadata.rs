use github_kit::{GitHubApiRequestOptions, GitHubRelease, fetch_latest_release};

use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;

pub(crate) async fn fetch_latest_release_metadata(
    client: &reqwest::Client,
    cfg: &InstallerRuntimeConfig,
    repo: &str,
) -> OperationResult<GitHubRelease> {
    fetch_latest_release(
        client,
        &cfg.github_releases.api_bases,
        repo,
        GitHubApiRequestOptions::new()
            .with_user_agent("toolchain-installer")
            .with_bearer_token(cfg.github_releases.token.as_deref()),
    )
    .await
    .map_err(|err| OperationError::download(err.to_string()))
}
