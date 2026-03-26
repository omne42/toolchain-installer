use std::collections::HashSet;
use std::time::Duration;

use github_kit::GitHubApiRequestOptions;

use crate::contracts::ExecutionRequest;

pub(crate) const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
pub(crate) const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 120;
pub(crate) const DEFAULT_PYPI_INDEX: &str = "https://pypi.org/simple";

#[derive(Debug, Clone)]
pub(crate) struct InstallerRuntimeConfig {
    pub(crate) github_releases: GitHubReleasePolicy,
    pub(crate) download_sources: DownloadSourcePolicy,
    pub(crate) package_indexes: PackageIndexPolicy,
    pub(crate) python_mirrors: PythonMirrorPolicy,
    pub(crate) gateway: GatewayRoutingPolicy,
    pub(crate) download: DownloadPolicy,
}

#[derive(Debug, Clone)]
pub(crate) struct GitHubReleasePolicy {
    pub(crate) api_bases: Vec<String>,
    pub(crate) token: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DownloadSourcePolicy {
    pub(crate) mirror_prefixes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PackageIndexPolicy {
    pub(crate) indexes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PythonMirrorPolicy {
    pub(crate) install_mirrors: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GatewayRoutingPolicy {
    pub(crate) base: Option<String>,
    pub(crate) country: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DownloadPolicy {
    pub(crate) http_timeout: Duration,
    pub(crate) max_download_bytes: Option<u64>,
}

impl InstallerRuntimeConfig {
    pub(crate) fn from_execution_request(request: &ExecutionRequest) -> Self {
        Self {
            github_releases: GitHubReleasePolicy::from_environment(),
            download_sources: DownloadSourcePolicy::from_execution_request(request),
            package_indexes: PackageIndexPolicy::from_execution_request(request),
            python_mirrors: PythonMirrorPolicy::from_execution_request(request),
            gateway: GatewayRoutingPolicy::from_execution_request(request),
            download: DownloadPolicy::from_execution_request(request),
        }
    }
}

impl GitHubReleasePolicy {
    fn from_environment() -> Self {
        let github_api_bases = parse_csv_env("TOOLCHAIN_INSTALLER_GITHUB_API_BASES");
        let github_api_bases = if github_api_bases.is_empty() {
            vec![DEFAULT_GITHUB_API_BASE.to_string()]
        } else {
            github_api_bases
        };
        let github_token = parse_nonempty_env("TOOLCHAIN_INSTALLER_GITHUB_TOKEN")
            .or_else(|| parse_nonempty_env("GITHUB_TOKEN"));

        Self {
            api_bases: github_api_bases,
            token: github_token,
        }
    }

    pub(crate) fn api_request_options(&self) -> GitHubApiRequestOptions<'_> {
        GitHubApiRequestOptions::new()
            .with_bearer_token(self.token.as_deref())
            .with_user_agent("toolchain-installer")
    }
}

impl DownloadSourcePolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mut mirror_prefixes = parse_csv_env("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES");
        for prefix in &request.mirror_prefixes {
            if !prefix.trim().is_empty() {
                mirror_prefixes.push(prefix.trim().to_string());
            }
        }

        Self {
            mirror_prefixes: dedupe_strings(mirror_prefixes),
        }
    }
}

impl PackageIndexPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mut indexes = parse_csv_env("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES");
        for index in &request.package_indexes {
            if !index.trim().is_empty() {
                indexes.push(index.trim().to_string());
            }
        }
        let mut indexes = dedupe_strings(indexes);
        if indexes.is_empty() {
            indexes.push(DEFAULT_PYPI_INDEX.to_string());
        }
        Self { indexes }
    }
}

impl PythonMirrorPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mut install_mirrors = parse_csv_env("TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS");
        for mirror in &request.python_install_mirrors {
            if !mirror.trim().is_empty() {
                install_mirrors.push(mirror.trim().to_string());
            }
        }
        Self {
            install_mirrors: dedupe_strings(install_mirrors),
        }
    }
}

impl GatewayRoutingPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let base = request
            .gateway_base
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("TOOLCHAIN_INSTALLER_GATEWAY_BASE")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });

        let country = request
            .country
            .as_ref()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("TOOLCHAIN_INSTALLER_COUNTRY")
                    .ok()
                    .map(|value| value.trim().to_ascii_uppercase())
                    .filter(|value| !value.is_empty())
            });

        Self { base, country }
    }

    pub(crate) fn use_for_git_release(&self) -> bool {
        self.base.is_some() && self.country.as_deref() == Some("CN")
    }
}

impl DownloadPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let http_timeout = std::env::var("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS));
        let max_download_bytes = request
            .max_download_bytes
            .filter(|value| *value > 0)
            .or_else(|| parse_positive_u64_env("TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES"));

        Self {
            http_timeout,
            max_download_bytes,
        }
    }
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut unique = HashSet::new();
    values
        .into_iter()
        .filter(|value| unique.insert(value.clone()))
        .collect()
}

fn parse_csv_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_positive_u64_env(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn parse_nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_source_policy_preserves_request_order_while_deduping() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            mirror_prefixes: vec![
                "https://mirror-b.example/".to_string(),
                "https://mirror-a.example/".to_string(),
                "https://mirror-b.example/".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        assert_eq!(
            cfg.download_sources.mirror_prefixes,
            vec![
                "https://mirror-b.example/".to_string(),
                "https://mirror-a.example/".to_string(),
            ]
        );
    }

    #[test]
    fn package_index_policy_preserves_request_order_while_deduping() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            package_indexes: vec![
                "https://mirror-b.example/simple".to_string(),
                "https://mirror-a.example/simple".to_string(),
                "https://mirror-b.example/simple".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        assert_eq!(
            cfg.package_indexes.indexes,
            vec![
                "https://mirror-b.example/simple".to_string(),
                "https://mirror-a.example/simple".to_string(),
            ]
        );
    }

    #[test]
    fn python_mirror_policy_preserves_request_order_while_deduping() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            python_install_mirrors: vec![
                "https://mirror-b.example/python".to_string(),
                "https://mirror-a.example/python".to_string(),
                "https://mirror-b.example/python".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        assert_eq!(
            cfg.python_mirrors.install_mirrors,
            vec![
                "https://mirror-b.example/python".to_string(),
                "https://mirror-a.example/python".to_string(),
            ]
        );
    }
}
