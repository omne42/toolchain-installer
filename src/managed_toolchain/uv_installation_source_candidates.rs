use http_kit::probe_http_endpoint_detailed;

use crate::contracts::BootstrapSourceKind;
use crate::download_sources::redact_source_url;
use crate::installer_runtime_config::InstallerRuntimeConfig;

#[derive(Debug, Clone)]
pub(super) struct InstallationSourceCandidate {
    pub(super) label: String,
    pub(super) env: Vec<(String, String)>,
    pub(super) probe_url: Option<String>,
    pub(super) source_kind: BootstrapSourceKind,
}

pub(super) fn python_installation_source_candidates(
    cfg: &InstallerRuntimeConfig,
) -> Vec<InstallationSourceCandidate> {
    let mut candidates = vec![InstallationSourceCandidate {
        label: "python:official".to_string(),
        env: Vec::new(),
        probe_url: None,
        source_kind: BootstrapSourceKind::Canonical,
    }];
    for mirror in &cfg.python_mirrors.install_mirrors {
        let mirror = mirror.trim();
        candidates.push(InstallationSourceCandidate {
            label: format!("python-mirror:{}", redact_source_url(mirror)),
            env: vec![("UV_PYTHON_INSTALL_MIRROR".to_string(), mirror.to_string())],
            probe_url: None,
            source_kind: BootstrapSourceKind::PythonMirror,
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
                source_kind: BootstrapSourceKind::PackageIndex,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer_runtime_config::{
        DownloadPolicy, DownloadSourcePolicy, GatewayRoutingPolicy, GitHubReleasePolicy,
        InstallerRuntimeConfig, PackageIndexPolicy, PythonMirrorPolicy,
    };
    use std::time::Duration;

    fn test_runtime_config() -> InstallerRuntimeConfig {
        InstallerRuntimeConfig {
            github_releases: GitHubReleasePolicy {
                api_bases: vec!["https://api.github.com".to_string()],
                token: None,
            },
            download_sources: DownloadSourcePolicy {
                mirror_prefixes: Vec::new(),
            },
            package_indexes: PackageIndexPolicy {
                indexes: vec!["https://pypi.org/simple".to_string()],
            },
            python_mirrors: PythonMirrorPolicy {
                install_mirrors: vec!["https://mirror.example/python".to_string()],
            },
            gateway: GatewayRoutingPolicy {
                base: None,
                country: None,
            },
            download: DownloadPolicy {
                http_timeout: Duration::from_secs(5),
                max_download_bytes: None,
            },
        }
    }

    #[test]
    fn python_installation_sources_distinguish_official_and_mirror_kinds() {
        let candidates = python_installation_source_candidates(&test_runtime_config());
        assert_eq!(candidates[0].label, "python:official");
        assert_eq!(candidates[0].source_kind, BootstrapSourceKind::Canonical);
        assert_eq!(
            candidates[1].label,
            "python-mirror:https://mirror.example/python"
        );
        assert_eq!(candidates[1].source_kind, BootstrapSourceKind::PythonMirror);
    }
}
