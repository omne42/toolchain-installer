use http_kit::probe_http_endpoint_detailed;

use crate::contracts::BootstrapSourceKind;
use crate::download_sources::redact_source_url;
use crate::installer_runtime_config::InstallerRuntimeConfig;

const UV_PYTHON_OFFICIAL_PROBE_URL: &str =
    "https://github.com/astral-sh/python-build-standalone/releases/latest";

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
        probe_url: Some(UV_PYTHON_OFFICIAL_PROBE_URL.to_string()),
        source_kind: BootstrapSourceKind::Canonical,
    }];
    for mirror in &cfg.python_mirrors.install_mirrors {
        let mirror = mirror.trim();
        candidates.push(InstallationSourceCandidate {
            label: format!("python-mirror:{}", redact_source_url(mirror)),
            env: vec![("UV_PYTHON_INSTALL_MIRROR".to_string(), mirror.to_string())],
            probe_url: http_probe_url(mirror),
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

fn http_probe_url(raw: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer_runtime_config::{
        DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS, DEFAULT_UV_TIMEOUT_SECONDS, DownloadPolicy,
        DownloadSourcePolicy, GatewayRoutingPolicy, GitHubReleasePolicy, HostRecipePolicy,
        InstallerRuntimeConfig, ManagedToolchainPolicy, PackageIndexPolicy, PythonMirrorPolicy,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
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
            host_recipes: HostRecipePolicy {
                timeout: Duration::from_secs(DEFAULT_HOST_RECIPE_TIMEOUT_SECONDS),
            },
            managed_toolchain: ManagedToolchainPolicy {
                uv_recipe_timeout: Duration::from_secs(DEFAULT_UV_TIMEOUT_SECONDS),
            },
        }
    }

    #[test]
    fn python_installation_sources_distinguish_official_and_mirror_kinds() {
        let candidates = python_installation_source_candidates(&test_runtime_config());
        assert_eq!(candidates[0].label, "python:official");
        assert_eq!(candidates[0].source_kind, BootstrapSourceKind::Canonical);
        assert_eq!(
            candidates[0].probe_url.as_deref(),
            Some(UV_PYTHON_OFFICIAL_PROBE_URL)
        );
        assert_eq!(
            candidates[1].label,
            "python-mirror:https://mirror.example/python"
        );
        assert_eq!(candidates[1].source_kind, BootstrapSourceKind::PythonMirror);
        assert_eq!(
            candidates[1].probe_url.as_deref(),
            Some("https://mirror.example/python")
        );
    }

    #[test]
    fn python_installation_sources_skip_http_probe_for_non_http_mirror() {
        let mut cfg = test_runtime_config();
        cfg.python_mirrors.install_mirrors = vec!["file:///tmp/python-mirror".to_string()];

        let candidates = python_installation_source_candidates(&cfg);

        assert_eq!(candidates[1].probe_url, None);
    }

    #[tokio::test]
    async fn prioritize_reachable_installation_sources_moves_reachable_http_candidates_first() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind reachable server");
        let addr = listener.local_addr().expect("listener addr");
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });
        let reachable = format!("http://{addr}/mirror");
        let unreachable = "http://127.0.0.1:9/unreachable".to_string();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("build client");

        let ordered = prioritize_reachable_installation_sources(
            &client,
            vec![
                InstallationSourceCandidate {
                    label: "python:official".to_string(),
                    env: Vec::new(),
                    probe_url: Some(unreachable),
                    source_kind: BootstrapSourceKind::Canonical,
                },
                InstallationSourceCandidate {
                    label: "python-mirror:reachable".to_string(),
                    env: Vec::new(),
                    probe_url: Some(reachable),
                    source_kind: BootstrapSourceKind::PythonMirror,
                },
            ],
        )
        .await;

        handle.join().expect("join reachable server");
        assert_eq!(ordered[0].label, "python-mirror:reachable");
        assert_eq!(ordered[1].label, "python:official");
    }
}
