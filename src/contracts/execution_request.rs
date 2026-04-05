use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionRequest {
    pub target_triple: Option<String>,
    pub managed_dir: Option<PathBuf>,
    pub plan_base_dir: Option<PathBuf>,
    pub mirror_prefixes: Vec<String>,
    pub package_indexes: Vec<String>,
    pub python_install_mirrors: Vec<String>,
    pub github_api_bases: Vec<String>,
    pub github_token: Option<String>,
    pub gateway_base: Option<String>,
    pub country: Option<String>,
    pub http_timeout_seconds: Option<u64>,
    pub max_download_bytes: Option<u64>,
    pub uv_timeout_seconds: Option<u64>,
}

impl ExecutionRequest {
    pub fn with_process_environment_fallbacks(mut self) -> Self {
        if self.mirror_prefixes.is_empty() {
            self.mirror_prefixes = parse_csv_env("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES");
        }
        if self.package_indexes.is_empty() {
            self.package_indexes = parse_csv_env("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES");
        }
        if self.python_install_mirrors.is_empty() {
            self.python_install_mirrors =
                parse_csv_env("TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS");
        }
        if self.github_api_bases.is_empty() {
            self.github_api_bases = parse_csv_env("TOOLCHAIN_INSTALLER_GITHUB_API_BASES");
        }
        if option_string_is_empty(self.github_token.as_deref()) {
            self.github_token = parse_nonempty_env("TOOLCHAIN_INSTALLER_GITHUB_TOKEN")
                .or_else(|| parse_nonempty_env("GITHUB_TOKEN"));
        }
        if option_string_is_empty(self.gateway_base.as_deref()) {
            self.gateway_base = parse_nonempty_env("TOOLCHAIN_INSTALLER_GATEWAY_BASE");
        }
        if option_string_is_empty(self.country.as_deref()) {
            self.country = parse_nonempty_env("TOOLCHAIN_INSTALLER_COUNTRY")
                .map(|value| value.to_ascii_uppercase());
        }
        if self.http_timeout_seconds.is_none() {
            self.http_timeout_seconds =
                parse_positive_u64_env("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS");
        }
        if self.max_download_bytes.is_none() {
            self.max_download_bytes =
                parse_positive_u64_env("TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES");
        }
        if self.uv_timeout_seconds.is_none() {
            self.uv_timeout_seconds =
                parse_positive_u64_env("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS");
        }
        self
    }
}

fn option_string_is_empty(value: Option<&str>) -> bool {
    value.is_none_or(|value| value.trim().is_empty())
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

#[derive(Debug, Clone, Default)]
pub struct BootstrapCommand {
    pub execution: ExecutionRequest,
    pub tools: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    use super::ExecutionRequest;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_env_var(name: &str, previous: Option<OsString>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(name, value) },
            None => unsafe { std::env::remove_var(name) },
        }
    }

    #[test]
    fn process_environment_fallbacks_fill_only_missing_fields() {
        let _guard = env_lock().lock().expect("env lock");
        let names = [
            "TOOLCHAIN_INSTALLER_MIRROR_PREFIXES",
            "TOOLCHAIN_INSTALLER_PACKAGE_INDEXES",
            "TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS",
            "TOOLCHAIN_INSTALLER_GITHUB_API_BASES",
            "TOOLCHAIN_INSTALLER_GITHUB_TOKEN",
            "GITHUB_TOKEN",
            "TOOLCHAIN_INSTALLER_GATEWAY_BASE",
            "TOOLCHAIN_INSTALLER_COUNTRY",
            "TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS",
            "TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES",
            "TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS",
        ];
        let previous = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_MIRROR_PREFIXES",
                "https://env.example/releases",
            );
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_PACKAGE_INDEXES",
                "https://env.example/simple",
            );
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS",
                "https://env.example/python",
            );
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_GITHUB_API_BASES",
                "https://api.env.example",
            );
            std::env::set_var("TOOLCHAIN_INSTALLER_GITHUB_TOKEN", "env-token");
            std::env::set_var("GITHUB_TOKEN", "fallback-token");
            std::env::set_var("TOOLCHAIN_INSTALLER_GATEWAY_BASE", "https://gateway.env");
            std::env::set_var("TOOLCHAIN_INSTALLER_COUNTRY", "cn");
            std::env::set_var("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS", "7");
            std::env::set_var("TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES", "11");
            std::env::set_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS", "13");
        }

        let request = ExecutionRequest {
            mirror_prefixes: vec!["https://cli.example/releases".to_string()],
            package_indexes: vec!["https://cli.example/simple".to_string()],
            python_install_mirrors: vec!["https://cli.example/python".to_string()],
            github_api_bases: vec!["https://api.cli.example".to_string()],
            github_token: Some("cli-token".to_string()),
            gateway_base: Some("https://gateway.cli".to_string()),
            country: Some("US".to_string()),
            http_timeout_seconds: Some(17),
            max_download_bytes: Some(19),
            uv_timeout_seconds: Some(23),
            ..ExecutionRequest::default()
        }
        .with_process_environment_fallbacks();

        for (name, value) in previous {
            restore_env_var(name, value);
        }

        assert_eq!(
            request.mirror_prefixes,
            vec!["https://cli.example/releases"]
        );
        assert_eq!(request.package_indexes, vec!["https://cli.example/simple"]);
        assert_eq!(
            request.python_install_mirrors,
            vec!["https://cli.example/python"]
        );
        assert_eq!(request.github_api_bases, vec!["https://api.cli.example"]);
        assert_eq!(request.github_token.as_deref(), Some("cli-token"));
        assert_eq!(request.gateway_base.as_deref(), Some("https://gateway.cli"));
        assert_eq!(request.country.as_deref(), Some("US"));
        assert_eq!(request.http_timeout_seconds, Some(17));
        assert_eq!(request.max_download_bytes, Some(19));
        assert_eq!(request.uv_timeout_seconds, Some(23));
    }

    #[test]
    fn process_environment_fallbacks_capture_missing_runtime_fields() {
        let _guard = env_lock().lock().expect("env lock");
        let names = [
            "TOOLCHAIN_INSTALLER_GITHUB_API_BASES",
            "TOOLCHAIN_INSTALLER_GITHUB_TOKEN",
            "GITHUB_TOKEN",
            "TOOLCHAIN_INSTALLER_GATEWAY_BASE",
            "TOOLCHAIN_INSTALLER_COUNTRY",
            "TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS",
            "TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES",
            "TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS",
        ];
        let previous = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        unsafe {
            std::env::set_var(
                "TOOLCHAIN_INSTALLER_GITHUB_API_BASES",
                "https://api-a.example, https://api-b.example",
            );
            std::env::remove_var("TOOLCHAIN_INSTALLER_GITHUB_TOKEN");
            std::env::set_var("GITHUB_TOKEN", "fallback-token");
            std::env::set_var("TOOLCHAIN_INSTALLER_GATEWAY_BASE", "https://gateway.env");
            std::env::set_var("TOOLCHAIN_INSTALLER_COUNTRY", "cn");
            std::env::set_var("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS", "29");
            std::env::set_var("TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES", "31");
            std::env::set_var("TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS", "37");
        }

        let request = ExecutionRequest::default().with_process_environment_fallbacks();

        for (name, value) in previous {
            restore_env_var(name, value);
        }

        assert_eq!(
            request.github_api_bases,
            vec![
                "https://api-a.example".to_string(),
                "https://api-b.example".to_string()
            ]
        );
        assert_eq!(request.github_token.as_deref(), Some("fallback-token"));
        assert_eq!(request.gateway_base.as_deref(), Some("https://gateway.env"));
        assert_eq!(request.country.as_deref(), Some("CN"));
        assert_eq!(request.http_timeout_seconds, Some(29));
        assert_eq!(request.max_download_bytes, Some(31));
        assert_eq!(request.uv_timeout_seconds, Some(37));
    }
}
