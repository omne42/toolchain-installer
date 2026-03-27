use http_kit::probe_http_endpoint_detailed;

use crate::installer_runtime_config::InstallerRuntimeConfig;

#[derive(Debug, Clone)]
pub(super) struct InstallationSourceCandidate {
    pub(super) label: String,
    pub(super) env: Vec<(String, String)>,
    pub(super) probe_url: Option<String>,
}

pub(super) fn python_installation_source_candidates(
    cfg: &InstallerRuntimeConfig,
) -> Vec<InstallationSourceCandidate> {
    let mut candidates = vec![InstallationSourceCandidate {
        label: "python-mirror:official".to_string(),
        env: Vec::new(),
        probe_url: None,
    }];
    for mirror in &cfg.python_mirrors.install_mirrors {
        let mirror = mirror.trim();
        candidates.push(InstallationSourceCandidate {
            label: format!("python-mirror:{}", redact_source_url(mirror)),
            env: vec![("UV_PYTHON_INSTALL_MIRROR".to_string(), mirror.to_string())],
            probe_url: None,
        });
    }
    candidates
}

pub(super) fn package_index_installation_source_candidates(
    cfg: &InstallerRuntimeConfig,
) -> Vec<InstallationSourceCandidate> {
    cfg.package_indexes
        .indexes
        .iter()
        .map(|index| {
            let index = index.trim();
            InstallationSourceCandidate {
                label: format!("package-index:{}", redact_source_url(index)),
                env: vec![("UV_DEFAULT_INDEX".to_string(), index.to_string())],
                probe_url: Some(index.to_string()),
            }
        })
        .collect()
}

pub(super) async fn prioritize_reachable_installation_sources(
    client: &reqwest::Client,
    candidates: Vec<InstallationSourceCandidate>,
) -> Vec<InstallationSourceCandidate> {
    let mut reachable = Vec::new();
    let mut deferred = Vec::new();
    for candidate in candidates {
        match candidate.probe_url.as_deref() {
            Some(url)
                if probe_http_endpoint_detailed(client, url)
                    .await
                    .is_reachable() =>
            {
                reachable.push(candidate)
            }
            Some(_) => deferred.push(candidate),
            None => reachable.push(candidate),
        }
    }
    reachable.extend(deferred);
    reachable
}

fn redact_source_url(raw: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return raw.to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}
