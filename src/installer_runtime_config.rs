use std::collections::HashSet;
use std::time::Duration;

use crate::contracts::ExecutionRequest;

pub(crate) const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
pub(crate) const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 120;
pub(crate) const DEFAULT_UV_TIMEOUT_SECONDS: u64 = 15 * 60;
pub(crate) const DEFAULT_PYPI_INDEX: &str = "https://pypi.org/simple";

#[derive(Debug, Clone)]
pub(crate) struct InstallerRuntimeConfig {
    pub(crate) github_releases: GitHubReleasePolicy,
    pub(crate) download_sources: DownloadSourcePolicy,
    pub(crate) package_indexes: PackageIndexPolicy,
    pub(crate) python_mirrors: PythonMirrorPolicy,
    pub(crate) gateway: GatewayRoutingPolicy,
    pub(crate) download: DownloadPolicy,
    pub(crate) managed_toolchain: ManagedToolchainPolicy,
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

#[derive(Debug, Clone)]
pub(crate) struct ManagedToolchainPolicy {
    pub(crate) uv_recipe_timeout: Duration,
}

impl InstallerRuntimeConfig {
    pub(crate) fn from_execution_request(request: &ExecutionRequest) -> Self {
        Self {
            github_releases: GitHubReleasePolicy::from_execution_request(request),
            download_sources: DownloadSourcePolicy::from_execution_request(request),
            package_indexes: PackageIndexPolicy::from_execution_request(request),
            python_mirrors: PythonMirrorPolicy::from_execution_request(request),
            gateway: GatewayRoutingPolicy::from_execution_request(request),
            download: DownloadPolicy::from_execution_request(request),
            managed_toolchain: ManagedToolchainPolicy::from_execution_request(request),
        }
    }
}

impl GitHubReleasePolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let api_bases = dedupe_strings(
            request
                .github_api_bases
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        );
        let api_bases = if api_bases.is_empty() {
            vec![DEFAULT_GITHUB_API_BASE.to_string()]
        } else {
            api_bases
        };

        Self {
            api_bases,
            token: request
                .github_token
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    }
}

impl DownloadSourcePolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mirror_prefixes = explicit_values(&request.mirror_prefixes);

        Self {
            mirror_prefixes: dedupe_strings(mirror_prefixes),
        }
    }
}

impl PackageIndexPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mut indexes = dedupe_strings(explicit_values(&request.package_indexes));
        if indexes.is_empty() {
            indexes.push(DEFAULT_PYPI_INDEX.to_string());
        }
        Self { indexes }
    }
}

impl PythonMirrorPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let install_mirrors = explicit_values(&request.python_install_mirrors);
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
            .filter(|value| !value.is_empty());

        let country = request
            .country
            .as_ref()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty());

        Self { base, country }
    }

    pub(crate) fn use_for_git_release(&self) -> bool {
        self.base.is_some() && self.country.as_deref() == Some("CN")
    }
}

impl DownloadPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let http_timeout = request
            .http_timeout_seconds
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS));
        let max_download_bytes = request.max_download_bytes.filter(|value| *value > 0);

        Self {
            http_timeout,
            max_download_bytes,
        }
    }
}

impl ManagedToolchainPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let uv_recipe_timeout = request
            .uv_timeout_seconds
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_UV_TIMEOUT_SECONDS));
        Self { uv_recipe_timeout }
    }
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut unique = HashSet::new();
    values
        .into_iter()
        .filter(|value| unique.insert(value.clone()))
        .collect()
}

fn explicit_values(explicit: &[String]) -> Vec<String> {
    explicit
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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

    #[test]
    fn explicit_mirror_prefixes_are_preserved() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            mirror_prefixes: vec![
                "https://cli-a.example/releases".to_string(),
                "https://cli-b.example/releases".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        assert_eq!(
            cfg.download_sources.mirror_prefixes,
            vec![
                "https://cli-a.example/releases".to_string(),
                "https://cli-b.example/releases".to_string(),
            ]
        );
    }

    #[test]
    fn explicit_package_indexes_are_preserved() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            package_indexes: vec![
                "https://cli-a.example/simple".to_string(),
                "https://cli-b.example/simple".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        assert_eq!(
            cfg.package_indexes.indexes,
            vec![
                "https://cli-a.example/simple".to_string(),
                "https://cli-b.example/simple".to_string(),
            ]
        );
    }

    #[test]
    fn explicit_python_mirrors_are_preserved() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            python_install_mirrors: vec![
                "https://cli-a.example/python".to_string(),
                "https://cli-b.example/python".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        assert_eq!(
            cfg.python_mirrors.install_mirrors,
            vec![
                "https://cli-a.example/python".to_string(),
                "https://cli-b.example/python".to_string(),
            ]
        );
    }

    #[test]
    fn managed_toolchain_policy_uses_request_timeout_override() {
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            uv_timeout_seconds: Some(7),
            ..ExecutionRequest::default()
        });
        assert_eq!(
            cfg.managed_toolchain.uv_recipe_timeout,
            Duration::from_secs(7)
        );
    }

    #[test]
    fn runtime_config_ignores_process_environment_once_request_is_built() {
        let _guard = env_lock().lock().expect("env lock");
        let request = ExecutionRequest {
            github_api_bases: vec!["https://api.request.example".to_string()],
            github_token: Some("request-token".to_string()),
            gateway_base: Some("https://gateway.request".to_string()),
            country: Some("US".to_string()),
            http_timeout_seconds: Some(41),
            max_download_bytes: Some(43),
            uv_timeout_seconds: Some(47),
            ..ExecutionRequest::default()
        };

        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_GITHUB_API_BASES",
                "https://api.env.example",
            );
            std::env::set_var("TOOLCHAIN_INSTALLER_GITHUB_TOKEN", "env-token");
            std::env::set_var("TOOLCHAIN_INSTALLER_GATEWAY_BASE", "https://gateway.env");
            std::env::set_var("TOOLCHAIN_INSTALLER_COUNTRY", "CN");
            std::env::set_var("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS", "53");
            std::env::set_var("TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES", "59");
            std::env::set_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS", "61");
        }

        let cfg = InstallerRuntimeConfig::from_execution_request(&request);

        unsafe {
            std::env::remove_var("TOOLCHAIN_INSTALLER_GITHUB_API_BASES");
            std::env::remove_var("TOOLCHAIN_INSTALLER_GITHUB_TOKEN");
            std::env::remove_var("TOOLCHAIN_INSTALLER_GATEWAY_BASE");
            std::env::remove_var("TOOLCHAIN_INSTALLER_COUNTRY");
            std::env::remove_var("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS");
            std::env::remove_var("TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES");
            std::env::remove_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS");
        }

        assert_eq!(
            cfg.github_releases.api_bases,
            vec!["https://api.request.example".to_string()]
        );
        assert_eq!(cfg.github_releases.token.as_deref(), Some("request-token"));
        assert_eq!(cfg.gateway.base.as_deref(), Some("https://gateway.request"));
        assert_eq!(cfg.gateway.country.as_deref(), Some("US"));
        assert_eq!(cfg.download.http_timeout, Duration::from_secs(41));
        assert_eq!(cfg.download.max_download_bytes, Some(43));
        assert_eq!(
            cfg.managed_toolchain.uv_recipe_timeout,
            Duration::from_secs(47)
        );
    }
}
