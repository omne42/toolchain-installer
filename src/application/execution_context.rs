use std::path::PathBuf;

use omne_host_info_primitives::{detect_host_target_triple, resolve_target_triple};

use crate::contracts::ExecutionRequest;
use crate::error::{InstallerError, InstallerResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;

pub(crate) struct ExecutionContext {
    pub(crate) host_triple: String,
    pub(crate) target_triple: String,
    pub(crate) managed_dir: PathBuf,
    pub(crate) cfg: InstallerRuntimeConfig,
    pub(crate) client: reqwest::Client,
}

impl ExecutionContext {
    pub(crate) fn for_bootstrap(request: &ExecutionRequest) -> InstallerResult<Self> {
        Self::build(request, TargetConstraint::HostOnly)
    }

    pub(crate) fn for_install_plan(request: &ExecutionRequest) -> InstallerResult<Self> {
        Self::build(request, TargetConstraint::AllowCrossTarget)
    }

    fn build(request: &ExecutionRequest, constraint: TargetConstraint) -> InstallerResult<Self> {
        let host_triple = detect_host_target_triple()
            .map(str::to_string)
            .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
        let target_triple = resolve_target_triple(request.target_triple.as_deref(), &host_triple);
        if matches!(constraint, TargetConstraint::HostOnly) && target_triple != host_triple {
            return Err(InstallerError::usage(format!(
                "bootstrap only supports the current host triple `{host_triple}`; use `--method release` or `--plan-file` for cross-target downloads"
            )));
        }
        let managed_dir =
            resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
                .ok_or_else(|| {
                    InstallerError::install("cannot resolve managed toolchain directory")
                })?;
        let cfg = InstallerRuntimeConfig::from_execution_request(request);
        let client = reqwest::Client::builder()
            // GitHub release asset transfers are more reliable via HTTP/1.1 in our CI/runtime mix.
            .http1_only()
            .timeout(cfg.download.http_timeout)
            .user_agent("toolchain-installer")
            .build()
            .map_err(|err| InstallerError::download(format!("build http client failed: {err}")))?;

        Ok(Self {
            host_triple,
            target_triple,
            managed_dir,
            cfg,
            client,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetConstraint {
    HostOnly,
    AllowCrossTarget,
}
