use std::path::{Path, PathBuf};

use omne_host_info_primitives::executable_suffix_for_target;

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::{command_path_exists, run_recipe_with_env};

#[derive(Clone, Copy)]
enum NpmManager {
    Npm,
    Pnpm,
    Bun,
}

struct NpmGlobalRecipe {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    binary_path: PathBuf,
    source: String,
}

pub(crate) fn execute_npm_global_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    let manager = parse_manager(item.manager.as_deref())?;
    let package = resolve_versioned_package(item)?;
    let binary_name = item
        .binary_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(item.id.as_str());

    let recipe =
        build_npm_global_recipe(manager, package, binary_name, target_triple, managed_dir)?;
    run_recipe_with_env(recipe.program.as_ref(), &recipe.args, &recipe.env)?;

    if !command_path_exists(&recipe.binary_path) {
        return Err(OperationError::install(format!(
            "expected npm_global binary at {}",
            recipe.binary_path.display()
        )));
    }

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(recipe.source),
        source_kind: None,
        archive_match: None,
        destination: Some(recipe.binary_path.display().to_string()),
        detail: None,
        error_code: None,
        failure_code: None,
    })
}

fn parse_manager(raw: Option<&str>) -> OperationResult<NpmManager> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(NpmManager::Npm),
        Some("npm") => Ok(NpmManager::Npm),
        Some("pnpm") => Ok(NpmManager::Pnpm),
        Some("bun") => Ok(NpmManager::Bun),
        Some(value) => Err(OperationError::install(format!(
            "unsupported npm_global manager `{value}`"
        ))),
    }
}

fn resolve_versioned_package(item: &InstallPlanItem) -> OperationResult<String> {
    let package = item
        .package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperationError::install("npm_global method requires `package`"))?;
    let version = item
        .version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(version) = version {
        if package.contains('@') || package.starts_with("file:") {
            return Ok(package.to_string());
        }
        return Ok(format!("{package}@{version}"));
    }
    Ok(package.to_string())
}

fn build_npm_global_recipe(
    manager: NpmManager,
    package: String,
    binary_name: &str,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<NpmGlobalRecipe> {
    let ext = executable_suffix_for_target(target_triple);
    match manager {
        NpmManager::Npm => {
            let prefix_root = npm_prefix_root_for_target(target_triple, managed_dir)?;
            let binary_path = if target_triple.contains("windows") {
                prefix_root.join(format!("{binary_name}.cmd"))
            } else {
                prefix_root.join("bin").join(binary_name)
            };
            Ok(NpmGlobalRecipe {
                program: "npm".to_string(),
                args: vec!["install".to_string(), "--global".to_string(), package],
                env: vec![(
                    "npm_config_prefix".to_string(),
                    prefix_root.display().to_string(),
                )],
                binary_path,
                source: "npm:npm".to_string(),
            })
        }
        NpmManager::Pnpm => {
            let binary_path = managed_dir.join(format!("{binary_name}{ext}"));
            Ok(NpmGlobalRecipe {
                program: "pnpm".to_string(),
                args: vec!["add".to_string(), "--global".to_string(), package],
                env: vec![("PNPM_HOME".to_string(), managed_dir.display().to_string())],
                binary_path,
                source: "npm:pnpm".to_string(),
            })
        }
        NpmManager::Bun => {
            let install_root = managed_dir.parent().unwrap_or(managed_dir);
            let binary_path = managed_dir.join(binary_name);
            Ok(NpmGlobalRecipe {
                program: "bun".to_string(),
                args: vec!["add".to_string(), "--global".to_string(), package],
                env: vec![(
                    "BUN_INSTALL".to_string(),
                    install_root.display().to_string(),
                )],
                binary_path,
                source: "npm:bun".to_string(),
            })
        }
    }
}

fn npm_prefix_root_for_target(target_triple: &str, managed_dir: &Path) -> OperationResult<PathBuf> {
    if target_triple.contains("windows") {
        return Ok(managed_dir.to_path_buf());
    }
    if managed_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == "bin")
    {
        return managed_dir
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| OperationError::install("cannot determine npm global prefix root"));
    }
    Ok(managed_dir.to_path_buf())
}
