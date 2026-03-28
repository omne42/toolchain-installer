use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use omne_host_info_primitives::{detect_host_platform, executable_suffix_for_target};
use omne_process_primitives::{
    HostRecipeRequest, resolve_command_path_or_standard_location, run_host_recipe,
};
use omne_system_package_primitives::default_system_package_install_recipes_for_os;

use crate::artifact::InstallSource;
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
    if host_command_is_healthy(tool) {
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
            let status = if is_supported_builtin_tool(tool) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManagedBootstrapState {
    NeedsInstall,
    ManagedHealthy { detail: String },
    ManagedBroken { detail: String },
}

pub(crate) fn assess_managed_bootstrap_state(
    tool: &str,
    target_triple: &str,
    destination: &Path,
    managed_dir: &Path,
) -> ManagedBootstrapState {
    if !destination.exists() {
        return ManagedBootstrapState::NeedsInstall;
    }

    if tool == "git" && target_triple.contains("windows") {
        return managed_windows_git_state(managed_dir);
    }

    if managed_binary_reports_version(destination) {
        return ManagedBootstrapState::ManagedHealthy {
            detail: "managed binary passed --version health check".to_string(),
        };
    }

    ManagedBootstrapState::ManagedBroken {
        detail: format!(
            "managed binary exists at {} but failed --version health check",
            destination.display()
        ),
    }
}

fn managed_windows_git_state(managed_dir: &Path) -> ManagedBootstrapState {
    let launcher_path = managed_dir.join("git.cmd");
    let portable_root = managed_dir.join("git-portable");
    let launcher = match std::fs::read_to_string(&launcher_path) {
        Ok(launcher) => launcher,
        Err(err) => {
            return ManagedBootstrapState::ManagedBroken {
                detail: format!(
                    "managed git launcher exists but cannot be read at {}: {err}",
                    launcher_path.display()
                ),
            };
        }
    };
    let Some(relative_target) = launcher_target_from_script(&launcher) else {
        return ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git launcher at {} does not point to a MinGit payload",
                launcher_path.display()
            ),
        };
    };
    let executable =
        match managed_windows_git_payload_path(managed_dir, &portable_root, &relative_target) {
            Ok(executable) => executable,
            Err(detail) => return ManagedBootstrapState::ManagedBroken { detail },
        };
    if !executable.exists() {
        return ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git launcher points to missing MinGit payload {}",
                executable.display()
            ),
        };
    }
    if let Some(expected_dll) = expected_mingit_runtime_dll(&relative_target) {
        let runtime_dll = managed_dir.join(expected_dll);
        if !runtime_dll.exists() {
            return ManagedBootstrapState::ManagedBroken {
                detail: format!(
                    "managed git payload is missing required runtime {}",
                    runtime_dll.display()
                ),
            };
        }
    }
    if !managed_binary_reports_version(&executable) {
        return ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git payload {} failed --version health check",
                executable.display()
            ),
        };
    }

    ManagedBootstrapState::ManagedHealthy {
        detail: format!(
            "managed git launcher points to healthy MinGit payload {} under {}",
            executable.display(),
            portable_root.display()
        ),
    }
}

fn launcher_target_from_script(script: &str) -> Option<PathBuf> {
    script.lines().find_map(|line| {
        let start = line.find("%~dp0")?;
        let rest = &line[start + 5..];
        let end = rest.find('"')?;
        let target = rest[..end].trim();
        if target.is_empty() {
            return None;
        }
        let mut relative = PathBuf::new();
        for component in target.split(['\\', '/']).filter(|part| !part.is_empty()) {
            relative.push(component);
        }
        (!relative.as_os_str().is_empty()).then_some(relative)
    })
}

fn managed_windows_git_payload_path(
    managed_dir: &Path,
    portable_root: &Path,
    relative_target: &Path,
) -> Result<PathBuf, String> {
    if relative_target.is_absolute()
        || relative_target.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "managed git launcher points outside managed root with payload target `{}`",
            relative_target.display()
        ));
    }
    let executable = managed_dir.join(relative_target);
    if !executable.starts_with(portable_root) {
        return Err(format!(
            "managed git launcher points outside managed git-portable root with payload target `{}`",
            relative_target.display()
        ));
    }
    Ok(executable)
}

fn expected_mingit_runtime_dll(relative_target: &Path) -> Option<PathBuf> {
    let normalized = relative_target.to_string_lossy().replace('\\', "/");
    if normalized.ends_with("PortableGit/mingw64/bin/git.exe")
        || normalized.ends_with("PortableGit/usr/bin/git.exe")
        || normalized.ends_with("PortableGit/bin/git.exe")
    {
        return relative_target
            .parent()
            .map(|parent| parent.join("msys-2.0.dll"));
    }
    None
}

fn managed_binary_reports_version(path: &Path) -> bool {
    let output = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    output.status.success()
}

pub(crate) fn host_command_is_healthy(tool: &str) -> bool {
    is_supported_builtin_tool(tool)
        && resolve_command_path_or_standard_location(tool)
            .is_some_and(|path| managed_binary_reports_version(&path))
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
            default_system_package_install_recipes_for_os(
                platform.operating_system().as_str(),
                "git",
            )
        })
        .unwrap_or_default();
    if recipes.is_empty() {
        return Err(OperationError::install(format!(
            "git install for target `{target_triple}` requires package manager but none is supported on this OS"
        )));
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        match run_host_recipe(&HostRecipeRequest::new(
            recipe.program.as_ref(),
            &recipe.args,
        )) {
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
