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
    for mirror in &cfg.python_install_mirrors {
        candidates.push(InstallationSourceCandidate {
            label: format!("python-mirror:{mirror}"),
            env: vec![(
                "UV_PYTHON_INSTALL_MIRROR".to_string(),
                mirror.trim().to_string(),
            )],
            probe_url: None,
        });
    }
    candidates
}

pub(super) fn package_index_installation_source_candidates(
    cfg: &InstallerRuntimeConfig,
) -> Vec<InstallationSourceCandidate> {
    cfg.package_indexes
        .iter()
        .map(|index| InstallationSourceCandidate {
            label: format!("package-index:{index}"),
            env: vec![("UV_DEFAULT_INDEX".to_string(), index.trim().to_string())],
            probe_url: Some(index.trim().to_string()),
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
