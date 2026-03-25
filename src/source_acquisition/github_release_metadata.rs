use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GithubReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    pub(crate) digest: Option<String>,
}

pub(crate) async fn fetch_latest_github_release(
    client: &reqwest::Client,
    github_api_bases: &[String],
    repo: &str,
) -> anyhow::Result<GithubRelease> {
    let mut errors = Vec::new();
    for base in github_api_bases {
        let trimmed = base.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let url = format!("{trimmed}/repos/{repo}/releases/latest");
        let mut request = client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, "toolchain-installer");
        if let Some(token) = github_api_token() {
            request = request.bearer_auth(token);
        }
        match request.send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    errors.push(format!("{url} -> HTTP {}", resp.status()));
                    continue;
                }
                match resp.json::<GithubRelease>().await {
                    Ok(release) => return Ok(release),
                    Err(err) => errors.push(format!("{url} -> invalid json: {err}")),
                }
            }
            Err(err) => errors.push(format!("{url} -> {err}")),
        }
    }

    Err(anyhow::anyhow!(
        "failed to fetch latest release metadata for {repo}: {}",
        errors.join(" | ")
    ))
}

fn github_api_token() -> Option<String> {
    std::env::var("TOOLCHAIN_INSTALLER_GITHUB_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("GITHUB_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
}
