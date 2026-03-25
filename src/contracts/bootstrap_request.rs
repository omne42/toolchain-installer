use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct BootstrapRequest {
    pub target_triple: Option<String>,
    pub managed_dir: Option<PathBuf>,
    pub tools: Vec<String>,
    pub mirror_prefixes: Vec<String>,
    pub package_indexes: Vec<String>,
    pub python_install_mirrors: Vec<String>,
    pub gateway_base: Option<String>,
    pub country: Option<String>,
    pub max_download_bytes: Option<u64>,
}
