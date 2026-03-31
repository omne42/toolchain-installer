use reqwest::Url;

use crate::installer_runtime_config::InstallerRuntimeConfig;

pub(crate) fn gateway_candidate_for_git_release_download_url(
    cfg: &InstallerRuntimeConfig,
    url: &str,
) -> Option<String> {
    let base = gateway_base_for_git_release(cfg)?;
    let (tag, asset) = git_release_asset_from_url(url)?;
    Some(make_gateway_asset_candidate(base, "git", &tag, &asset))
}

fn git_release_asset_from_url(url: &str) -> Option<(String, String)> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https" || parsed.query().is_some() || parsed.fragment().is_some() {
        return None;
    }
    if parsed.host_str()? != "github.com" {
        return None;
    }
    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    if segments.len() != 6
        || segments[0] != "git-for-windows"
        || segments[1] != "git"
        || segments[2] != "releases"
        || segments[3] != "download"
    {
        return None;
    }
    let tag = segments[4];
    let asset = segments[5];
    if tag.is_empty() || asset.is_empty() {
        return None;
    }
    Some((tag.to_string(), asset.to_string()))
}

pub(crate) fn gateway_candidate_for_git_release_asset(
    cfg: &InstallerRuntimeConfig,
    tag: &str,
    asset_name: &str,
) -> Option<String> {
    let base = gateway_base_for_git_release(cfg)?;
    let safe_tag = tag.trim();
    let safe_asset_name = asset_name.trim();
    if safe_tag.is_empty() || safe_asset_name.is_empty() {
        return None;
    }
    Some(make_gateway_asset_candidate(
        base,
        "git",
        safe_tag,
        safe_asset_name,
    ))
}

fn gateway_base_for_git_release(cfg: &InstallerRuntimeConfig) -> Option<&str> {
    cfg.gateway
        .use_for_git_release()
        .then_some(cfg.gateway.base.as_deref())
        .flatten()
}

pub(crate) fn make_gateway_asset_candidate(
    base: &str,
    tool: &str,
    tag: &str,
    asset_name: &str,
) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    let safe_tag = tag.trim();
    format!("{trimmed}/toolchain/{tool}/{safe_tag}/{asset_name}")
}

#[cfg(test)]
mod tests {
    use super::git_release_asset_from_url;

    #[test]
    fn git_release_asset_from_url_accepts_exact_github_release_download_path() {
        assert_eq!(
            git_release_asset_from_url(
                "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip"
            ),
            Some((
                "v2.48.1.windows.1".to_string(),
                "MinGit-2.48.1-busybox-64-bit.zip".to_string()
            ))
        );
    }

    #[test]
    fn git_release_asset_from_url_rejects_non_github_hosts_and_embedded_substrings() {
        assert_eq!(
            git_release_asset_from_url(
                "https://mirror.example/proxy/github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip"
            ),
            None
        );
        assert_eq!(
            git_release_asset_from_url(
                "https://example.com/?next=/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip"
            ),
            None
        );
    }

    #[test]
    fn git_release_asset_from_url_rejects_non_https_and_query_variants() {
        assert_eq!(
            git_release_asset_from_url(
                "http://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip"
            ),
            None
        );
        assert_eq!(
            git_release_asset_from_url(
                "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip?download=1"
            ),
            None
        );
        assert_eq!(
            git_release_asset_from_url(
                "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip#fragment"
            ),
            None
        );
    }

    #[test]
    fn git_release_asset_from_url_rejects_other_repositories() {
        assert_eq!(
            git_release_asset_from_url(
                "https://github.com/cli/cli/releases/download/v2.0.0/gh_2.0.0_linux_amd64.tar.gz"
            ),
            None
        );
    }
}
