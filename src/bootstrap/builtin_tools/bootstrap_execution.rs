use std::path::{Path, PathBuf};

use omne_host_info_primitives::{
    detect_host_target_triple, executable_suffix_for_target, resolve_target_triple,
};
use omne_system_package_primitives::default_system_package_install_recipes_for_current_host;

use crate::contracts::{
    BootstrapItem, BootstrapRequest, BootstrapResult, BootstrapSourceKind, BootstrapStatus,
    InstallSource, OUTPUT_SCHEMA_VERSION,
};
use crate::error::{InstallerError, InstallerResult, OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::managed_root_dir::resolve_managed_toolchain_dir;
use crate::platform::process_runner::{command_available, run_recipe};
use crate::uv::release_installation::install_uv_from_public;

use super::builtin_tool_selection::{is_supported_builtin_tool, normalize_requested_tools};
use super::public_release_asset_installation::{
    install_gh_from_public_release, install_git_from_public_release,
};

pub async fn bootstrap(request: &BootstrapRequest) -> InstallerResult<BootstrapResult> {
    let host_triple = detect_host_target_triple()
        .map(str::to_string)
        .ok_or_else(|| InstallerError::install("unsupported host platform/arch"))?;
    let target_triple = resolve_target_triple(request.target_triple.as_deref(), &host_triple);
    if target_triple != host_triple {
        return Err(InstallerError::usage(format!(
            "bootstrap only supports the current host triple `{host_triple}`; use `--method release` or `--plan-file` for cross-target downloads"
        )));
    }
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| InstallerError::install("cannot resolve managed toolchain directory"))?;
    let cfg = InstallerRuntimeConfig::from_request(request);
    let client = reqwest::Client::builder()
        // GitHub release asset transfers are more reliable via HTTP/1.1 in our CI/runtime mix.
        .http1_only()
        .timeout(cfg.http_timeout)
        .user_agent("toolchain-installer")
        .build()
        .map_err(|err| InstallerError::download(format!("build http client failed: {err}")))?;
    let binary_ext = executable_suffix_for_target(&target_triple);

    let tools = normalize_requested_tools(&request.tools);
    let mut items = Vec::new();
    for tool in tools {
        let item = bootstrap_builtin_tool(
            tool.as_str(),
            &target_triple,
            binary_ext,
            &managed_dir,
            &cfg,
            &client,
        )
        .await;
        items.push(item);
    }

    Ok(BootstrapResult {
        schema_version: OUTPUT_SCHEMA_VERSION,
        host_triple,
        target_triple,
        managed_dir: managed_dir.display().to_string(),
        items,
    })
}

async fn bootstrap_builtin_tool(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> BootstrapItem {
    if command_available(tool) {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Present,
            source: None,
            source_kind: None,
            archive_match: None,
            destination: None,
            detail: None,
            error_code: None,
            failure_code: None,
        };
    }

    let destination = bootstrap_destination(tool, target_triple, binary_ext, managed_dir);
    if destination.exists() {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Installed,
            source: Some("managed".to_string()),
            source_kind: Some(BootstrapSourceKind::Managed),
            archive_match: None,
            destination: Some(destination.display().to_string()),
            detail: Some("managed binary already exists".to_string()),
            error_code: None,
            failure_code: None,
        };
    }

    match install_builtin_tool(tool, target_triple, binary_ext, &destination, cfg, client).await {
        Ok(source) => {
            let InstallSource {
                locator,
                source_kind,
                archive_match,
            } = source;
            BootstrapItem {
                tool: tool.to_string(),
                status: BootstrapStatus::Installed,
                source: Some(locator),
                source_kind: Some(source_kind),
                archive_match,
                destination: Some(destination.display().to_string()),
                detail: None,
                error_code: None,
                failure_code: None,
            }
        }
        Err(err) => {
            let status = if is_supported_builtin_tool(tool) {
                BootstrapStatus::Failed
            } else {
                BootstrapStatus::Unsupported
            };
            BootstrapItem {
                tool: tool.to_string(),
                status,
                source: None,
                source_kind: None,
                archive_match: None,
                destination: Some(destination.display().to_string()),
                detail: Some(err.message),
                error_code: (status == BootstrapStatus::Failed)
                    .then(|| crate::error::error_code_label(err.exit_code).to_string()),
                failure_code: (status == BootstrapStatus::Failed).then_some(err.exit_code),
            }
        }
    }
}

fn bootstrap_destination(
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
        "uv" => install_uv_from_public(target_triple, destination, cfg, client).await,
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

    install_git_via_system_package_manager(target_triple)
}

fn install_git_via_system_package_manager(target_triple: &str) -> OperationResult<InstallSource> {
    let recipes = default_system_package_install_recipes_for_current_host("git");
    if recipes.is_empty() {
        return Err(OperationError::install(format!(
            "git install for target `{target_triple}` requires package manager but none is supported on this OS"
        )));
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        match run_recipe(recipe.program, &recipe.args) {
            Ok(_) => {
                if command_available("git") {
                    return Ok(InstallSource::new(
                        format!("system:{}", recipe.program),
                        BootstrapSourceKind::SystemPackage,
                    ));
                }
                errors.push(format!(
                    "{} succeeded but `git --version` is still unavailable",
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
