use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct ExecutionRequest {
    pub target_triple: Option<String>,
    pub managed_dir: Option<PathBuf>,
    pub mirror_prefixes: Vec<String>,
    pub package_indexes: Vec<String>,
    pub python_install_mirrors: Vec<String>,
    pub gateway_base: Option<String>,
    pub country: Option<String>,
    pub max_download_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct BootstrapCommand {
    pub execution: ExecutionRequest,
    pub tools: Vec<String>,
}
