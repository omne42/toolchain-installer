use std::path::{Path, PathBuf};

use omne_fs_primitives::{AdvisoryLockGuard, lock_advisory_file_in_ambient_root};
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
    _managed_dir_lock: AdvisoryLockGuard,
}

impl ExecutionContext {
    pub(crate) fn for_bootstrap(request: &ExecutionRequest) -> InstallerResult<Self> {
        Self::build(request, TargetConstraint::HostOnly)
    }

    #[cfg_attr(not(test), allow(dead_code))]
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
        let managed_dir_lock = acquire_managed_dir_execution_lock(&managed_dir)?;
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
            _managed_dir_lock: managed_dir_lock,
        })
    }
}

pub(crate) fn acquire_managed_dir_execution_lock(
    managed_dir: &Path,
) -> InstallerResult<AdvisoryLockGuard> {
    let lock_root = managed_dir.parent().unwrap_or_else(|| Path::new("."));
    let lock_file = managed_dir_lock_file_name(managed_dir);
    lock_advisory_file_in_ambient_root(
        lock_root,
        "managed dir lock root",
        Path::new(&lock_file),
        "managed dir lock file",
    )
    .map_err(|err| {
        InstallerError::install(format!(
            "cannot lock managed_dir {} for exclusive execution: {err}",
            managed_dir.display()
        ))
    })
}

fn managed_dir_lock_file_name(managed_dir: &Path) -> String {
    let label = managed_dir
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_lock_component)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "managed-dir".to_string());
    format!(".toolchain-installer-managed-dir-{label}.lock")
}

fn sanitize_lock_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetConstraint {
    HostOnly,
    AllowCrossTarget,
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use omne_fs_primitives::lock_advisory_file_in_ambient_root;

    use super::{ExecutionContext, managed_dir_lock_file_name};
    use crate::contracts::ExecutionRequest;

    #[test]
    fn execution_context_holds_exclusive_lock_for_managed_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let request = ExecutionRequest {
            managed_dir: Some(managed_dir.clone()),
            ..ExecutionRequest::default()
        };

        let ctx = ExecutionContext::for_install_plan(&request).expect("execution context");
        let lock_root = managed_dir
            .parent()
            .expect("managed_dir parent")
            .to_path_buf();
        let lock_file = managed_dir_lock_file_name(&managed_dir);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _guard = lock_advisory_file_in_ambient_root(
                &lock_root,
                "managed dir lock root",
                Path::new(&lock_file),
                "managed dir lock file",
            )
            .expect("competing managed_dir lock");
            sender.send(()).expect("send lock acquired");
        });

        assert!(
            receiver.recv_timeout(Duration::from_millis(100)).is_err(),
            "execution context should hold the managed_dir lock while it is alive"
        );

        drop(ctx);

        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("managed_dir lock should be released after context drop");
        worker.join().expect("worker thread");
    }
}
