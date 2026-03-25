use crate::installer_runtime_config::InstallerRuntimeConfig;

pub(crate) fn infer_gateway_candidate_for_git_release(
    cfg: &InstallerRuntimeConfig,
    url: &str,
) -> Option<String> {
    let base = cfg.gateway_base.as_deref()?;
    let marker = "/git-for-windows/git/releases/download/";
    let index = url.find(marker)?;
    let suffix = &url[(index + marker.len())..];
    let mut segments = suffix.split('/');
    let tag = segments.next()?;
    let asset = segments.next()?;
    if tag.is_empty() || asset.is_empty() {
        return None;
    }
    Some(make_gateway_asset_candidate(base, "git", tag, asset))
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
