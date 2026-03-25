use std::collections::BTreeSet;
use std::time::Duration;

use crate::contracts::BootstrapRequest;

pub(crate) const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
pub(crate) const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 20;
pub(crate) const DEFAULT_PYPI_INDEX: &str = "https://pypi.org/simple";

#[derive(Debug, Clone)]
pub(crate) struct InstallerRuntimeConfig {
    pub(crate) github_api_bases: Vec<String>,
    pub(crate) mirror_prefixes: Vec<String>,
    pub(crate) package_indexes: Vec<String>,
    pub(crate) python_install_mirrors: Vec<String>,
    pub(crate) gateway_base: Option<String>,
    pub(crate) country: Option<String>,
    pub(crate) http_timeout: Duration,
    pub(crate) max_download_bytes: Option<u64>,
}

impl InstallerRuntimeConfig {
    pub(crate) fn from_request(request: &BootstrapRequest) -> Self {
        let github_api_bases = parse_csv_env("TOOLCHAIN_INSTALLER_GITHUB_API_BASES");
        let github_api_bases = if github_api_bases.is_empty() {
            vec![DEFAULT_GITHUB_API_BASE.to_string()]
        } else {
            github_api_bases
        };

        let mut mirror_prefixes = parse_csv_env("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES");
        for prefix in &request.mirror_prefixes {
            if !prefix.trim().is_empty() {
                mirror_prefixes.push(prefix.trim().to_string());
            }
        }
        let mut unique = BTreeSet::new();
        mirror_prefixes.retain(|value| unique.insert(value.clone()));

        let mut package_indexes = vec![DEFAULT_PYPI_INDEX.to_string()];
        package_indexes.extend(parse_csv_env("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES"));
        for index in &request.package_indexes {
            if !index.trim().is_empty() {
                package_indexes.push(index.trim().to_string());
            }
        }
        let mut unique = BTreeSet::new();
        package_indexes.retain(|value| unique.insert(value.clone()));

        let mut python_install_mirrors =
            parse_csv_env("TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS");
        for mirror in &request.python_install_mirrors {
            if !mirror.trim().is_empty() {
                python_install_mirrors.push(mirror.trim().to_string());
            }
        }
        let mut unique = BTreeSet::new();
        python_install_mirrors.retain(|value| unique.insert(value.clone()));

        let gateway_base = request
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
            github_api_bases,
            mirror_prefixes,
            package_indexes,
            python_install_mirrors,
            gateway_base,
            country,
            http_timeout,
            max_download_bytes,
        }
    }

    pub(crate) fn use_gateway_for_git_release(&self) -> bool {
        self.gateway_base.is_some() && self.country.as_deref() == Some("CN")
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
