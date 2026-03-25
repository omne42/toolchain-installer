use std::path::{Path, PathBuf};

use crate::contracts::{BootstrapItem, BootstrapStatus, InstallPlanItem};
use crate::error::{OperationError, OperationResult};
use crate::platform::process_runner::{
    command_path_exists, resolve_command_for_execution, run_recipe_with_env,
};

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

    let recipe = build_npm_global_recipe(
        manager,
        package.clone(),
        binary_name,
        target_triple,
        managed_dir,
    )?;
    run_recipe_with_env(recipe.program.as_ref(), &recipe.args, &recipe.env)?;

    let destination = resolve_npm_global_destination(
        &recipe.binary_path,
        &package,
        binary_name,
        Some(managed_dir),
    )
    .ok_or_else(|| {
        OperationError::install(format!(
            "expected npm_global binary at {}",
            recipe.binary_path.display()
        ))
    })?;

    if !command_path_exists(&destination) {
        return Err(OperationError::install(format!(
            "expected npm_global binary at {}",
            destination.display()
        )));
    }

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(recipe.source),
        source_kind: None,
        archive_match: None,
        destination: Some(destination.display().to_string()),
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
    match manager {
        NpmManager::Npm => {
            let prefix_root = npm_prefix_root_for_target(target_triple, managed_dir)?;
            let binary_path = if target_triple.contains("windows") {
                prefix_root.join(global_binary_filename(binary_name, manager, target_triple))
            } else {
                prefix_root.join("bin").join(binary_name)
            };
            Ok(NpmGlobalRecipe {
                program: resolve_command_for_execution("npm"),
                args: vec![
                    "install".to_string(),
                    "--global".to_string(),
                    "--prefix".to_string(),
                    prefix_root.display().to_string(),
                    package,
                ],
                env: vec![(
                    "npm_config_prefix".to_string(),
                    prefix_root.display().to_string(),
                )],
                binary_path,
                source: "npm:npm".to_string(),
            })
        }
        NpmManager::Pnpm => {
            let binary_path =
                managed_dir.join(global_binary_filename(binary_name, manager, target_triple));
            Ok(NpmGlobalRecipe {
                program: resolve_command_for_execution("pnpm"),
                args: vec!["add".to_string(), "--global".to_string(), package],
                env: vec![
                    ("PNPM_HOME".to_string(), managed_dir.display().to_string()),
                    ("PATH".to_string(), prepend_path_env(managed_dir)?),
                ],
                binary_path,
                source: "npm:pnpm".to_string(),
            })
        }
        NpmManager::Bun => {
            let global_dir = managed_dir.join("install").join("global");
            let binary_dir = managed_dir.join("bin");
            let binary_path =
                binary_dir.join(global_binary_filename(binary_name, manager, target_triple));
            Ok(NpmGlobalRecipe {
                program: resolve_command_for_execution("bun"),
                args: vec!["add".to_string(), "--global".to_string(), package],
                env: vec![
                    (
                        "BUN_INSTALL_GLOBAL_DIR".to_string(),
                        global_dir.display().to_string(),
                    ),
                    (
                        "BUN_INSTALL_BIN".to_string(),
                        binary_dir.display().to_string(),
                    ),
                    ("PATH".to_string(), prepend_path_env(&binary_dir)?),
                ],
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

fn resolve_npm_global_destination(
    binary_path: &Path,
    package: &str,
    binary_name: &str,
    search_root: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(destination) = find_binary_at_path(binary_path, binary_name) {
        return Some(destination);
    }

    let prefix_root = binary_path.parent()?.parent()?;
    let mut package_dir = prefix_root.join("lib").join("node_modules");
    for segment in npm_package_name(package).split('/') {
        package_dir.push(segment);
    }
    if let Some(destination) = find_named_binary_under_dir(&package_dir, binary_name) {
        return Some(destination);
    }

    search_root.and_then(|root| find_named_binary_under_dir(root, binary_name))
}

fn npm_package_name(package: &str) -> &str {
    let package = package.trim();
    if package.starts_with('@') {
        if let Some((name, _version)) = package.rsplit_once('@')
            && name.contains('/')
        {
            return name;
        }
        return package;
    }
    package
        .split_once('@')
        .map(|(name, _)| name)
        .unwrap_or(package)
}

fn find_named_binary_under_dir(root: &Path, binary_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| binary_name_matches(value, binary_name))
                && command_path_exists(&path)
            {
                return Some(path);
            }
        }
    }
    None
}

fn prepend_path_env(path: &Path) -> OperationResult<String> {
    let mut entries = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(entries)
        .map_err(|err| OperationError::install(format!("cannot compose PATH: {err}")))?;
    Ok(joined.to_string_lossy().into_owned())
}

fn global_binary_filename(binary_name: &str, manager: NpmManager, target_triple: &str) -> String {
    if target_triple.contains("windows") {
        let extension = match manager {
            NpmManager::Npm | NpmManager::Pnpm | NpmManager::Bun => ".cmd",
        };
        return format!("{binary_name}{extension}");
    }
    binary_name.to_string()
}

fn find_binary_at_path(binary_path: &Path, binary_name: &str) -> Option<PathBuf> {
    if command_path_exists(binary_path) {
        return Some(binary_path.to_path_buf());
    }

    let parent = binary_path.parent()?;
    for candidate_name in candidate_binary_names(binary_name) {
        let candidate = parent.join(candidate_name);
        if command_path_exists(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn candidate_binary_names(binary_name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        vec![
            binary_name.to_string(),
            format!("{binary_name}.cmd"),
            format!("{binary_name}.exe"),
            format!("{binary_name}.bat"),
            format!("{binary_name}.ps1"),
        ]
    }
    #[cfg(not(windows))]
    {
        vec![binary_name.to_string()]
    }
}

fn binary_name_matches(candidate: &str, binary_name: &str) -> bool {
    candidate_binary_names(binary_name)
        .iter()
        .any(|value| value == candidate)
}

#[cfg(test)]
mod tests {
    use super::{NpmManager, build_npm_global_recipe};

    #[test]
    fn pnpm_recipe_prepends_pnpm_home_to_path() {
        let managed_dir = std::env::temp_dir().join("ti-pnpm-home");
        let recipe = build_npm_global_recipe(
            NpmManager::Pnpm,
            "http-server@14.1.1".to_string(),
            "http-server",
            host_target_triple(),
            &managed_dir,
        )
        .expect("build pnpm recipe");

        assert!(recipe.env.iter().any(
            |(name, value)| name == "PNPM_HOME" && value == &managed_dir.display().to_string()
        ));
        let path = recipe
            .env
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.as_str())
            .expect("PATH env");
        let first = std::env::split_paths(std::ffi::OsStr::new(path))
            .next()
            .expect("first PATH entry");
        assert_eq!(first, managed_dir);
    }

    #[test]
    fn bun_recipe_configures_global_and_bin_dirs() {
        let managed_dir = std::env::temp_dir().join("ti-bun-root");
        let recipe = build_npm_global_recipe(
            NpmManager::Bun,
            "http-server@14.1.1".to_string(),
            "http-server",
            host_target_triple(),
            &managed_dir,
        )
        .expect("build bun recipe");

        let expected_global_dir = managed_dir.join("install").join("global");
        assert!(recipe.env.iter().any(|(name, value)| {
            name == "BUN_INSTALL_GLOBAL_DIR" && value == &expected_global_dir.display().to_string()
        }));
        let expected_binary_dir = managed_dir.join("bin");
        assert!(
            recipe
                .env
                .iter()
                .any(|(name, value)| name == "BUN_INSTALL_BIN"
                    && value == &expected_binary_dir.display().to_string())
        );
        assert_eq!(
            recipe.binary_path.parent(),
            Some(expected_binary_dir.as_path())
        );
        let path = recipe
            .env
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.as_str())
            .expect("PATH env");
        let first = std::env::split_paths(std::ffi::OsStr::new(path))
            .next()
            .expect("first PATH entry");
        assert_eq!(first, expected_binary_dir);
    }

    fn host_target_triple() -> &'static str {
        #[cfg(windows)]
        {
            "x86_64-pc-windows-msvc"
        }
        #[cfg(target_os = "macos")]
        {
            "x86_64-apple-darwin"
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            "x86_64-unknown-linux-gnu"
        }
    }
}
