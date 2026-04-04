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
            github_releases: GitHubReleasePolicy::from_environment(),
            download_sources: DownloadSourcePolicy::from_execution_request(request),
            package_indexes: PackageIndexPolicy::from_execution_request(request),
            python_mirrors: PythonMirrorPolicy::from_execution_request(request),
            gateway: GatewayRoutingPolicy::from_execution_request(request),
            download: DownloadPolicy::from_execution_request(request),
            managed_toolchain: ManagedToolchainPolicy::from_environment(),
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
}

impl DownloadSourcePolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mirror_prefixes = explicit_or_env_csv(
            &request.mirror_prefixes,
            "TOOLCHAIN_INSTALLER_MIRROR_PREFIXES",
        );

        Self {
            mirror_prefixes: dedupe_strings(mirror_prefixes),
        }
    }
}

impl PackageIndexPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let mut indexes = dedupe_strings(explicit_or_env_csv(
            &request.package_indexes,
            "TOOLCHAIN_INSTALLER_PACKAGE_INDEXES",
        ));
        if indexes.is_empty() {
            indexes.push(DEFAULT_PYPI_INDEX.to_string());
        }
        Self { indexes }
    }
}

impl PythonMirrorPolicy {
    fn from_execution_request(request: &ExecutionRequest) -> Self {
        let install_mirrors = explicit_or_env_csv(
            &request.python_install_mirrors,
            "TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS",
        );
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

impl ManagedToolchainPolicy {
    fn from_environment() -> Self {
        let uv_recipe_timeout = parse_positive_u64_env("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS")
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

fn explicit_or_env_csv(explicit: &[String], env_name: &str) -> Vec<String> {
    let explicit = explicit
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if explicit.is_empty() {
        parse_csv_env(env_name)
    } else {
        explicit
    }
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
    fn explicit_mirror_prefixes_override_environment_values() {
        let _guard = env_lock().lock().expect("env lock");
        let previous = std::env::var_os("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES");
        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_MIRROR_PREFIXES",
                "https://env.example/releases,https://env-second.example/releases",
            );
        }

        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            mirror_prefixes: vec![
                "https://cli-a.example/releases".to_string(),
                "https://cli-b.example/releases".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        restore_env_var("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES", previous);
        assert_eq!(
            cfg.download_sources.mirror_prefixes,
            vec![
                "https://cli-a.example/releases".to_string(),
                "https://cli-b.example/releases".to_string(),
            ]
        );
    }

    #[test]
    fn explicit_package_indexes_override_environment_values() {
        let _guard = env_lock().lock().expect("env lock");
        let previous = std::env::var_os("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES");
        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_PACKAGE_INDEXES",
                "https://env.example/simple,https://env-second.example/simple",
            );
        }

        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            package_indexes: vec![
                "https://cli-a.example/simple".to_string(),
                "https://cli-b.example/simple".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        restore_env_var("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES", previous);
        assert_eq!(
            cfg.package_indexes.indexes,
            vec![
                "https://cli-a.example/simple".to_string(),
                "https://cli-b.example/simple".to_string(),
            ]
        );
    }

    #[test]
    fn explicit_python_mirrors_override_environment_values() {
        let _guard = env_lock().lock().expect("env lock");
        let previous = std::env::var_os("TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS");
        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS",
                "https://env.example/python,https://env-second.example/python",
            );
        }

        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
            python_install_mirrors: vec![
                "https://cli-a.example/python".to_string(),
                "https://cli-b.example/python".to_string(),
            ],
            ..ExecutionRequest::default()
        });

        restore_env_var("TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS", previous);
        assert_eq!(
            cfg.python_mirrors.install_mirrors,
            vec![
                "https://cli-a.example/python".to_string(),
                "https://cli-b.example/python".to_string(),
            ]
        );
    }

    #[test]
    fn managed_toolchain_policy_uses_uv_timeout_override() {
        let _guard = env_lock().lock().expect("env lock");
        let previous = std::env::var_os("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS");
        unsafe {
            std::env::set_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS", "7");
        }

        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest::default());

        match previous {
            Some(value) => unsafe {
                std::env::set_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS", value)
            },
            None => unsafe { std::env::remove_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS") },
        }

        assert_eq!(
            cfg.managed_toolchain.uv_recipe_timeout,
            Duration::from_secs(7)
        );
    }

    fn restore_env_var(name: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(name, value) },
            None => unsafe { std::env::remove_var(name) },
        }
    }
}
