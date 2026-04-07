use std::ffi::OsString;
use std::path::{Path, PathBuf};

use omne_host_info_primitives::detect_host_platform;
use omne_process_primitives::{HostRecipeRequest, resolve_command_path_or_standard_location};
use omne_system_package_primitives::try_default_system_package_install_recipes_for_os;

use crate::artifact::InstallSource;
use crate::builtin_tools::bootstrap_tool_health::{
    ManagedBootstrapState, assess_managed_bootstrap_state, host_command_is_healthy,
    host_command_is_healthy_including_standard_locations,
};
use crate::builtin_tools::builtin_tool_selection::is_supported_builtin_tool;
use crate::builtin_tools::public_release_asset_installation::{
    install_gh_from_public_release, install_git_from_public_release,
};
use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::host_recipe::run_installer_host_recipe;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::install_uv_from_public_release;

pub(crate) fn builtin_tool_destination(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    managed_dir: &Path,
) -> PathBuf {
    if tool == "git" && target_triple.contains("windows") {
        return managed_dir.join("git.cmd");
    }
    managed_dir.join(format!("{tool}{binary_ext}"))
}

pub(crate) async fn bootstrap_builtin_tool(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    destination: &Path,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> BootstrapItem {
    let supported_tool = is_supported_builtin_tool(tool);
    let managed_state =
        assess_managed_bootstrap_state(tool, target_triple, destination, managed_dir);
    if let Some(item) = reusable_bootstrap_item(
        tool,
        destination,
        &managed_state,
        supported_tool && host_command_is_healthy(tool),
    ) {
        return item;
    }

    match install_builtin_tool(tool, target_triple, binary_ext, destination, cfg, client).await {
        Ok(source) => {
            if let Err(err) = verify_bootstrap_installation(
                tool,
                target_triple,
                destination,
                managed_dir,
                source.source_kind,
            ) {
                let status = if supported_tool {
                    BootstrapStatus::Failed
                } else {
                    BootstrapStatus::Unsupported
                };
                let detail = match &managed_state {
                    ManagedBootstrapState::ManagedBroken {
                        detail: broken_detail,
                    } => format!("{broken_detail}; reinstall failed: {}", err.detail()),
                    _ => err.detail(),
                };
                let error_code =
                    if matches!(managed_state, ManagedBootstrapState::ManagedBroken { .. }) {
                        "managed_install_broken".to_string()
                    } else {
                        err.error_code().to_string()
                    };
                let exit_code = err.exit_code();
                return BootstrapItem {
                    tool: tool.to_string(),
                    status,
                    source: None,
                    source_kind: None,
                    archive_match: None,
                    destination: Some(destination.display().to_string()),
                    detail: Some(detail),
                    error_code: (status == BootstrapStatus::Failed).then_some(error_code),
                    failure_code: (status == BootstrapStatus::Failed).then_some(exit_code),
                };
            }
            let InstallSource {
                locator,
                source_kind,
                archive_match,
            } = source;
            let destination = resolved_bootstrap_destination(tool, destination, source_kind);
            let detail = match managed_state {
                ManagedBootstrapState::ManagedBroken { detail } => Some(format!(
                    "reinstalled after broken managed install: {detail}"
                )),
                _ => None,
            };
            BootstrapItem {
                tool: tool.to_string(),
                status: BootstrapStatus::Installed,
                source: Some(locator),
                source_kind: Some(source_kind),
                archive_match,
                destination,
                detail,
                error_code: None,
                failure_code: None,
            }
        }
        Err(err) => {
            let status = if supported_tool {
                BootstrapStatus::Failed
            } else {
                BootstrapStatus::Unsupported
            };
            let detail = match &managed_state {
                ManagedBootstrapState::ManagedBroken {
                    detail: broken_detail,
                } => format!("{broken_detail}; reinstall failed: {}", err.detail()),
                _ => err.detail(),
            };
            let error_code = if matches!(managed_state, ManagedBootstrapState::ManagedBroken { .. })
            {
                "managed_install_broken".to_string()
            } else {
                err.error_code().to_string()
            };
            let exit_code = err.exit_code();
            BootstrapItem {
                tool: tool.to_string(),
                status,
                source: None,
                source_kind: None,
                archive_match: None,
                destination: Some(destination.display().to_string()),
                detail: Some(detail),
                error_code: (status == BootstrapStatus::Failed).then_some(error_code),
                failure_code: (status == BootstrapStatus::Failed).then_some(exit_code),
            }
        }
    }
}

fn verify_bootstrap_installation(
    tool: &str,
    target_triple: &str,
    destination: &Path,
    managed_dir: &Path,
    source_kind: BootstrapSourceKind,
) -> OperationResult<()> {
    if source_kind == BootstrapSourceKind::SystemPackage {
        if host_command_is_healthy_including_standard_locations(tool) {
            return Ok(());
        }
        return Err(OperationError::install(format!(
            "bootstrap install reported success but `{tool}` still failed the post-install health check"
        )));
    }

    match assess_managed_bootstrap_state(tool, target_triple, destination, managed_dir) {
        ManagedBootstrapState::ManagedHealthy { .. } => Ok(()),
        ManagedBootstrapState::NeedsInstall => Err(OperationError::install(format!(
            "bootstrap install reported success but managed destination {} is still missing",
            destination.display()
        ))),
        ManagedBootstrapState::ManagedBroken { detail } => Err(OperationError::install(format!(
            "bootstrap install reported success but managed install is unhealthy: {detail}"
        ))),
    }
}

fn reusable_bootstrap_item(
    tool: &str,
    destination: &Path,
    managed_state: &ManagedBootstrapState,
    host_is_healthy: bool,
) -> Option<BootstrapItem> {
    match managed_state {
        ManagedBootstrapState::ManagedHealthy { detail } => Some(BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Installed,
            source: Some("managed".to_string()),
            source_kind: Some(BootstrapSourceKind::Managed),
            archive_match: None,
            destination: Some(destination.display().to_string()),
            detail: Some(detail.clone()),
            error_code: None,
            failure_code: None,
        }),
        ManagedBootstrapState::NeedsInstall if host_is_healthy => Some(BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Present,
            source: None,
            source_kind: None,
            archive_match: None,
            destination: None,
            detail: None,
            error_code: None,
            failure_code: None,
        }),
        ManagedBootstrapState::NeedsInstall | ManagedBootstrapState::ManagedBroken { .. } => None,
    }
}

fn resolved_bootstrap_destination(
    tool: &str,
    preferred_destination: &Path,
    source_kind: BootstrapSourceKind,
) -> Option<String> {
    if source_kind == BootstrapSourceKind::SystemPackage {
        return resolve_command_path_or_standard_location(tool)
            .map(|path| path.display().to_string())
            .or_else(|| Some(preferred_destination.display().to_string()));
    }
    Some(preferred_destination.display().to_string())
}

async fn install_builtin_tool(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    match tool {
        "gh" => {
            install_gh_from_public_release(target_triple, binary_ext, destination, cfg, client)
                .await
        }
        "git" => install_git_for_bootstrap(target_triple, destination, cfg, client).await,
        "uv" => install_uv_from_public_release(target_triple, destination, cfg, client).await,
        _ => Err(OperationError::install(format!(
            "unsupported tool `{tool}`"
        ))),
    }
}

async fn install_git_for_bootstrap(
    target_triple: &str,
    destination: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> OperationResult<InstallSource> {
    if target_triple == "x86_64-pc-windows-msvc" || target_triple == "aarch64-pc-windows-msvc" {
        return install_git_from_public_release(target_triple, destination, cfg, client).await;
    }

    install_git_via_system_package_manager(target_triple, cfg)
}

fn install_git_via_system_package_manager(
    target_triple: &str,
    cfg: &InstallerRuntimeConfig,
) -> OperationResult<InstallSource> {
    let recipes = detect_host_platform()
        .map(|platform| {
            try_default_system_package_install_recipes_for_os(
                platform.operating_system().as_str(),
                "git",
            )
        })
        .transpose()
        .map_err(|err| OperationError::install(format!("invalid bootstrap package `git`: {err}")))?
        .unwrap_or_default();
    if recipes.is_empty() {
        return Err(OperationError::install(format!(
            "git install for target `{target_triple}` requires package manager but none is supported on this OS"
        )));
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        let args = recipe.args.iter().map(OsString::from).collect::<Vec<_>>();
        match run_installer_host_recipe(
            &HostRecipeRequest::new(recipe.program.as_ref(), &args),
            cfg.host_recipes.timeout,
        ) {
            Ok(_) => {
                if host_command_is_healthy_including_standard_locations("git") {
                    return Ok(InstallSource::new(
                        format!("system:{}", recipe.program),
                        BootstrapSourceKind::SystemPackage,
                    ));
                }
                errors.push(format!(
                    "{} succeeded but `git --version` is still unavailable from PATH or trusted standard locations",
                    recipe.program
                ));
            }
            Err(err) => errors.push(format!("{} failed: {err}", recipe.program)),
        }
    }

    Err(OperationError::install(format!(
        "all system package manager recipes failed: {}",
        errors.join(" | ")
    )))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{bootstrap_builtin_tool, verify_bootstrap_installation};
    use crate::contracts::{BootstrapSourceKind, BootstrapStatus, ExecutionRequest};
    use crate::installer_runtime_config::InstallerRuntimeConfig;

    fn write_executable(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)
                .expect("stat executable")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).expect("chmod executable");
        }
    }

    #[cfg_attr(windows, ignore = "mock executable is unix-specific")]
    #[test]
    fn verify_bootstrap_installation_accepts_healthy_managed_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let destination = managed_dir.join("uv");
        write_executable(
            &destination,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.8.0"
  exit 0
fi
exit 1
"#,
        );

        verify_bootstrap_installation(
            "uv",
            "x86_64-unknown-linux-gnu",
            &destination,
            &managed_dir,
            BootstrapSourceKind::Canonical,
        )
        .expect("healthy managed bootstrap install");
    }

    #[cfg_attr(windows, ignore = "mock executable is unix-specific")]
    #[test]
    fn verify_bootstrap_installation_rejects_broken_managed_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let destination = managed_dir.join("uv");
        write_executable(
            &destination,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "not uv"
  exit 0
fi
exit 1
"#,
        );

        let err = verify_bootstrap_installation(
            "uv",
            "x86_64-unknown-linux-gnu",
            &destination,
            &managed_dir,
            BootstrapSourceKind::Canonical,
        )
        .expect_err("broken managed bootstrap install should fail");

        assert!(
            err.detail()
                .contains("bootstrap install reported success but managed install is unhealthy")
        );
    }

    #[cfg_attr(windows, ignore = "mock executable is unix-specific")]
    #[tokio::test]
    async fn bootstrap_keeps_unsupported_status_even_with_healthy_managed_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let managed_dir = temp.path().join("managed");
        let destination = managed_dir.join("custom-tool");
        write_executable(
            &destination,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "custom-tool 1.0.0"
  exit 0
fi
exit 1
"#,
        );
        let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest::default());
        let client = reqwest::Client::builder()
            .build()
            .expect("build reqwest client");

        let item = bootstrap_builtin_tool(
            "custom-tool",
            "x86_64-unknown-linux-gnu",
            "",
            &destination,
            &managed_dir,
            &cfg,
            &client,
        )
        .await;

        assert_eq!(item.status, BootstrapStatus::Unsupported);
        assert_eq!(item.source, None);
        assert_eq!(
            item.detail,
            Some("unsupported tool `custom-tool`".to_string())
        );
    }
}
