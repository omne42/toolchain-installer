use std::ffi::OsString;
use std::path::{Path, PathBuf};

use omne_host_info_primitives::{detect_host_platform, executable_suffix_for_target};
use omne_process_primitives::{
    HostRecipeRequest, resolve_command_path_or_standard_location, run_host_recipe,
};
use omne_system_package_primitives::try_default_system_package_install_recipes_for_os;

use crate::artifact::InstallSource;
use crate::builtin_tools::bootstrap_tool_health::{
    ManagedBootstrapState, assess_managed_bootstrap_state, host_command_is_healthy,
};
use crate::builtin_tools::builtin_tool_selection::{
    is_supported_builtin_tool, normalize_requested_tools,
};
use crate::builtin_tools::public_release_asset_installation::{
    install_gh_from_public_release, install_git_from_public_release,
};
use crate::contracts::{
    BootstrapCommand, BootstrapItem, BootstrapResult, BootstrapSourceKind, BootstrapStatus,
    OUTPUT_SCHEMA_VERSION,
};
use crate::error::{InstallerResult, OperationError, OperationResult};
use crate::install_plan::item_destination_resolution::validate_managed_path_boundary;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use crate::managed_toolchain::install_uv_from_public_release;

use super::execution_context::ExecutionContext;

pub async fn bootstrap(command: &BootstrapCommand) -> InstallerResult<BootstrapResult> {
    let ctx = ExecutionContext::for_bootstrap(&command.execution)?;
    let binary_ext = executable_suffix_for_target(&ctx.target_triple);

    let tools = normalize_requested_tools(&command.tools);
    let mut items = Vec::new();
    for tool in tools {
        let item = bootstrap_builtin_tool(
            tool.as_str(),
            &ctx.target_triple,
            binary_ext,
            &ctx.managed_dir,
            &ctx.cfg,
            &ctx.client,
        )
        .await;
        items.push(item);
    }

    Ok(BootstrapResult {
        schema_version: OUTPUT_SCHEMA_VERSION,
        host_triple: ctx.host_triple,
        target_triple: ctx.target_triple,
        managed_dir: ctx.managed_dir.display().to_string(),
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
    let supported_tool = is_supported_builtin_tool(tool);
    if supported_tool && host_command_is_healthy(tool) {
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
    if let Err(detail) = validate_managed_path_boundary(&destination, managed_dir, false) {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Failed,
            source: None,
            source_kind: None,
            archive_match: None,
            destination: Some(destination.display().to_string()),
            detail: Some(detail),
            error_code: Some("install_failed".to_string()),
            failure_code: Some(crate::error::ExitCode::Install),
        };
    }
    let managed_state =
        assess_managed_bootstrap_state(tool, target_triple, &destination, managed_dir);
    if let ManagedBootstrapState::ManagedHealthy { detail } = &managed_state {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Installed,
            source: Some("managed".to_string()),
            source_kind: Some(BootstrapSourceKind::Managed),
            archive_match: None,
            destination: Some(destination.display().to_string()),
            detail: Some(detail.clone()),
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
            let destination = resolved_bootstrap_destination(tool, &destination, source_kind);
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

    install_git_via_system_package_manager(target_triple)
}

fn install_git_via_system_package_manager(target_triple: &str) -> OperationResult<InstallSource> {
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
        match run_host_recipe(&HostRecipeRequest::new(recipe.program.as_ref(), &args)) {
            Ok(_) => {
                if host_command_is_healthy("git") {
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
