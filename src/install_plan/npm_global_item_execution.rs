use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use omne_process_primitives::{
    HostRecipeRequest, command_path_exists, resolve_command_path, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{NodePackageManager, NpmGlobalPlanItem};

struct NpmGlobalRecipe {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    binary_path: PathBuf,
    fallback_package_dir: Option<PathBuf>,
    fallback_search_root: Option<PathBuf>,
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    modified: Option<SystemTime>,
    len: u64,
}

pub(crate) fn execute_npm_global_item(
    item: &NpmGlobalPlanItem,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<BootstrapItem> {
    let recipe = build_npm_global_recipe(
        item.manager,
        item.package_spec.clone(),
        &item.binary_name,
        target_triple,
        managed_dir,
    )?;
    let preinstall_state = capture_installation_state(
        &recipe.binary_path,
        &item.package_spec,
        &item.binary_name,
        recipe.fallback_package_dir.as_deref(),
        recipe.fallback_search_root.as_deref(),
    );
    run_host_recipe(
        &HostRecipeRequest::new(recipe.program.as_ref(), &recipe.args).with_env(&recipe.env),
    )
    .map_err(OperationError::from_host_recipe)?;

    let destination = match resolve_npm_global_destination(
        &recipe.binary_path,
        &item.package_spec,
        &item.binary_name,
        recipe.fallback_package_dir.as_deref(),
        recipe.fallback_search_root.as_deref(),
    ) {
        Some(destination) => destination,
        None => create_windows_bun_global_launcher(
            item.manager,
            &recipe.program,
            managed_dir,
            &item.package_spec,
            &item.binary_name,
        )?
        .ok_or_else(|| {
            OperationError::install(format!(
                "expected npm_global binary at {}",
                recipe.binary_path.display()
            ))
        })?,
    };
    if !installation_result_is_acceptable(
        &preinstall_state,
        &recipe.binary_path,
        &destination,
        &item.package_spec,
        &item.binary_name,
        recipe.fallback_package_dir.as_deref(),
        recipe.fallback_search_root.as_deref(),
    ) {
        return Err(OperationError::install(format!(
            "npm_global install for `{}` did not update the expected binary path {}; refusing to treat a stale managed file as a fresh install",
            item.package_spec,
            destination.display()
        )));
    }

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

fn build_npm_global_recipe(
    manager: NodePackageManager,
    package: String,
    binary_name: &str,
    target_triple: &str,
    managed_dir: &Path,
) -> OperationResult<NpmGlobalRecipe> {
    match manager {
        NodePackageManager::Npm => {
            let prefix_root = npm_prefix_root_for_target(target_triple, managed_dir)?;
            let fallback_package_dir = npm_global_package_dir(&prefix_root, &package);
            let binary_path = if target_triple.contains("windows") {
                prefix_root.join(global_binary_filename(binary_name, manager, target_triple))
            } else {
                prefix_root.join("bin").join(binary_name)
            };
            Ok(NpmGlobalRecipe {
                program: resolve_command_path("npm")
                    .and_then(|path| path.into_os_string().into_string().ok())
                    .unwrap_or_else(|| "npm".to_string()),
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
                fallback_package_dir: Some(fallback_package_dir),
                fallback_search_root: None,
                source: "npm:npm".to_string(),
            })
        }
        NodePackageManager::Pnpm => {
            let binary_path =
                managed_dir.join(global_binary_filename(binary_name, manager, target_triple));
            Ok(NpmGlobalRecipe {
                program: resolve_command_path("pnpm")
                    .and_then(|path| path.into_os_string().into_string().ok())
                    .unwrap_or_else(|| "pnpm".to_string()),
                args: vec!["add".to_string(), "--global".to_string(), package],
                env: vec![
                    ("PNPM_HOME".to_string(), managed_dir.display().to_string()),
                    ("PATH".to_string(), prepend_path_env(managed_dir)?),
                ],
                binary_path,
                fallback_package_dir: None,
                fallback_search_root: None,
                source: "npm:pnpm".to_string(),
            })
        }
        NodePackageManager::Bun => {
            let global_dir = managed_dir.join("install").join("global");
            let binary_dir = managed_dir.join("bin");
            let binary_path =
                binary_dir.join(global_binary_filename(binary_name, manager, target_triple));
            Ok(NpmGlobalRecipe {
                program: resolve_command_path("bun")
                    .and_then(|path| path.into_os_string().into_string().ok())
                    .unwrap_or_else(|| "bun".to_string()),
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
                fallback_package_dir: None,
                fallback_search_root: Some(global_dir.join("node_modules").join(".bin")),
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
    fallback_package_dir: Option<&Path>,
    fallback_search_root: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(destination) = find_binary_at_path(binary_path, binary_name) {
        return Some(destination);
    }

    if let Some(destination) = fallback_package_dir
        .and_then(|package_dir| resolve_package_bin_script(package_dir, package, binary_name))
        .filter(|path| command_path_exists(path))
    {
        return Some(destination);
    }

    if let Some(destination) =
        fallback_search_root.and_then(|root| find_named_binary_under_dir(root, binary_name))
    {
        return Some(destination);
    }

    None
}

fn capture_installation_state(
    binary_path: &Path,
    package: &str,
    binary_name: &str,
    fallback_package_dir: Option<&Path>,
    fallback_search_root: Option<&Path>,
) -> HashMap<PathBuf, Option<FileFingerprint>> {
    let mut paths = candidate_binary_paths(binary_path, binary_name);
    if let Some(package_dir) = fallback_package_dir {
        let manifest_path = package_dir.join("package.json");
        paths.push(manifest_path.clone());
        if let Some(destination) = resolve_package_bin_script(package_dir, package, binary_name) {
            paths.push(destination);
        }
    }
    if let Some(root) = fallback_search_root {
        paths.extend(find_matching_binary_paths_under_dir(root, binary_name));
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let fingerprint = file_fingerprint(&path);
            (path, fingerprint)
        })
        .collect()
}

fn installation_result_is_acceptable(
    preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>,
    binary_path: &Path,
    destination: &Path,
    package: &str,
    binary_name: &str,
    fallback_package_dir: Option<&Path>,
    fallback_search_root: Option<&Path>,
) -> bool {
    if path_changed(preinstall_state, destination) {
        return true;
    }

    if destination_preexisted(preinstall_state, destination)
        && candidate_binary_paths(binary_path, binary_name)
            .iter()
            .any(|candidate| candidate == destination)
    {
        return true;
    }

    if let Some(package_dir) = fallback_package_dir
        && resolve_package_bin_script(package_dir, package, binary_name).as_deref()
            == Some(destination)
    {
        return path_changed(preinstall_state, &package_dir.join("package.json"))
            || destination_preexisted(preinstall_state, destination);
    }

    if let Some(root) = fallback_search_root
        && destination.starts_with(root)
    {
        return destination_preexisted(preinstall_state, destination);
    }

    false
}

fn destination_preexisted(
    preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>,
    destination: &Path,
) -> bool {
    preinstall_state
        .get(destination)
        .is_some_and(|fingerprint| fingerprint.is_some())
        && command_path_exists(destination)
}

fn path_changed(preinstall_state: &HashMap<PathBuf, Option<FileFingerprint>>, path: &Path) -> bool {
    let Some(current) = file_fingerprint(path) else {
        return false;
    };
    match preinstall_state.get(path) {
        Some(Some(previous)) => previous != &current,
        Some(None) | None => true,
    }
}

fn npm_global_package_dir(prefix_root: &Path, package: &str) -> PathBuf {
    let mut package_dir = prefix_root.join("lib").join("node_modules");
    for segment in npm_package_name(package).split('/') {
        package_dir.push(segment);
    }
    package_dir
}

#[cfg(windows)]
fn create_windows_bun_global_launcher(
    manager: NodePackageManager,
    bun_program: &str,
    managed_dir: &Path,
    package: &str,
    binary_name: &str,
) -> OperationResult<Option<PathBuf>> {
    if !matches!(manager, NodePackageManager::Bun) {
        return Ok(None);
    }

    let global_dir = bun_global_dir(managed_dir);
    let package_dir = resolve_bun_package_dir(&global_dir, package).ok_or_else(|| {
        OperationError::install(format!(
            "cannot locate bun global package `{}` under {}",
            npm_package_name(package),
            global_dir.display()
        ))
    })?;
    let script_path = resolve_bun_package_bin_script(&package_dir, package, binary_name)
        .ok_or_else(|| {
            OperationError::install(format!(
                "cannot resolve bun global binary `{binary_name}` for package `{}`",
                npm_package_name(package)
            ))
        })?;
    if !script_path.exists() {
        return Err(OperationError::install(format!(
            "bun global binary script does not exist at {}",
            script_path.display()
        )));
    }

    let launcher_path = managed_dir.join("bin").join(format!("{binary_name}.cmd"));
    if let Some(parent) = launcher_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| OperationError::install(err.to_string()))?;
    }
    let launcher_body = format!(
        "@echo off\r\n\"{}\" \"{}\" %*\r\n",
        bun_program,
        script_path.display()
    );
    std::fs::write(&launcher_path, launcher_body)
        .map_err(|err| OperationError::install(err.to_string()))?;
    Ok(Some(launcher_path))
}

#[cfg(not(windows))]
fn create_windows_bun_global_launcher(
    _manager: NodePackageManager,
    _bun_program: &str,
    _managed_dir: &Path,
    _package: &str,
    _binary_name: &str,
) -> OperationResult<Option<PathBuf>> {
    Ok(None)
}

#[cfg(windows)]
fn bun_global_dir(managed_dir: &Path) -> PathBuf {
    managed_dir.join("install").join("global")
}

#[cfg(windows)]
fn resolve_bun_package_dir(global_dir: &Path, package: &str) -> Option<PathBuf> {
    let mut direct = global_dir.join("node_modules");
    for segment in npm_package_name(package).split('/') {
        direct.push(segment);
    }
    if direct.exists() {
        return Some(direct);
    }
    find_package_dir_under_root(global_dir, npm_package_name(package))
}

#[cfg(windows)]
fn find_package_dir_under_root(root: &Path, package_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let manifest_path = dir.join("package.json");
        if manifest_path.is_file() {
            let manifest = std::fs::read_to_string(&manifest_path).ok()?;
            if package_name_from_manifest(&manifest).is_some_and(|name| name == package_name) {
                return Some(dir);
            }
        }

        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

#[cfg(windows)]
fn resolve_bun_package_bin_script(
    package_dir: &Path,
    package: &str,
    binary_name: &str,
) -> Option<PathBuf> {
    let manifest_path = package_dir.join("package.json");
    let manifest = std::fs::read_to_string(&manifest_path).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let relative = package_bin_relative_path(&manifest, package, binary_name)?;
    Some(package_dir.join(relative))
}

fn package_bin_relative_path(
    manifest: &serde_json::Value,
    package: &str,
    binary_name: &str,
) -> Option<PathBuf> {
    let package_name = npm_package_name(package);
    let package_basename = package_name.rsplit('/').next().unwrap_or(package_name);
    let bin = manifest.get("bin")?;
    match bin {
        serde_json::Value::String(path) => sanitize_package_bin_relative_path(path),
        serde_json::Value::Object(entries) => {
            if let Some(path) = entries
                .get(binary_name)
                .or_else(|| entries.get(package_basename))
                .and_then(|value| value.as_str())
            {
                return sanitize_package_bin_relative_path(path);
            }

            if entries.len() == 1 {
                return entries
                    .values()
                    .next()
                    .and_then(|value| value.as_str())
                    .and_then(sanitize_package_bin_relative_path);
            }
            None
        }
        _ => None,
    }
}

fn sanitize_package_bin_relative_path(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => sanitized.push(segment),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    (!sanitized.as_os_str().is_empty()).then_some(sanitized)
}

fn resolve_package_bin_script(
    package_dir: &Path,
    package: &str,
    binary_name: &str,
) -> Option<PathBuf> {
    let manifest = std::fs::read_to_string(package_dir.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let relative = package_bin_relative_path(&manifest, package, binary_name)?;
    Some(package_dir.join(relative))
}

#[cfg(windows)]
fn package_name_from_manifest(manifest: &str) -> Option<String> {
    let manifest: serde_json::Value = serde_json::from_str(manifest).ok()?;
    manifest
        .get("name")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
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
    find_matching_binary_paths_under_dir(root, binary_name)
        .into_iter()
        .find(|path| command_path_exists(path))
}

fn find_matching_binary_paths_under_dir(root: &Path, binary_name: &str) -> Vec<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| binary_name_matches(value, binary_name))
            {
                matches.push(path);
            }
        }
    }
    matches.sort();
    matches
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

fn global_binary_filename(
    binary_name: &str,
    manager: NodePackageManager,
    target_triple: &str,
) -> String {
    if target_triple.contains("windows") {
        let extension = match manager {
            NodePackageManager::Npm | NodePackageManager::Pnpm | NodePackageManager::Bun => ".cmd",
        };
        return format!("{binary_name}{extension}");
    }
    binary_name.to_string()
}

fn find_binary_at_path(binary_path: &Path, binary_name: &str) -> Option<PathBuf> {
    candidate_binary_paths(binary_path, binary_name)
        .into_iter()
        .find(|candidate| candidate.is_file() && command_path_exists(candidate))
}

fn candidate_binary_paths(binary_path: &Path, binary_name: &str) -> Vec<PathBuf> {
    let mut paths = vec![binary_path.to_path_buf()];
    let Some(parent) = binary_path.parent() else {
        return paths;
    };
    for candidate_name in candidate_binary_names(binary_name) {
        let candidate = parent.join(candidate_name);
        if !paths.contains(&candidate) {
            paths.push(candidate);
        }
    }
    paths
}

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileFingerprint {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
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
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::{
        build_npm_global_recipe, capture_installation_state, file_fingerprint, find_binary_at_path,
        installation_result_is_acceptable, npm_global_package_dir, package_bin_relative_path,
        resolve_npm_global_destination, resolve_package_bin_script,
    };
    use crate::plan_items::NodePackageManager;

    #[test]
    fn pnpm_recipe_prepends_pnpm_home_to_path() {
        let managed_dir = std::env::temp_dir().join("ti-pnpm-home");
        let recipe = build_npm_global_recipe(
            NodePackageManager::Pnpm,
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
            NodePackageManager::Bun,
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

    #[test]
    fn package_bin_relative_path_supports_string_bin_field() {
        let manifest = json!({
            "name": "http-server",
            "bin": "bin/http-server"
        });

        let path = package_bin_relative_path(&manifest, "http-server@14.1.1", "http-server")
            .expect("bin path");
        assert_eq!(path, PathBuf::from("bin/http-server"));
    }

    #[test]
    fn package_bin_relative_path_supports_object_bin_field() {
        let manifest = json!({
            "name": "@scope/http-server",
            "bin": {
                "http-server": "dist/http-server.js",
                "other": "dist/other.js"
            }
        });

        let path = package_bin_relative_path(&manifest, "@scope/http-server@14.1.1", "http-server")
            .expect("bin path");
        assert_eq!(path, PathBuf::from("dist/http-server.js"));
    }

    #[test]
    fn package_bin_relative_path_rejects_parent_escape() {
        let manifest = json!({
            "name": "http-server",
            "bin": "../outside/http-server"
        });

        assert!(
            package_bin_relative_path(&manifest, "http-server@14.1.1", "http-server").is_none()
        );
    }

    #[test]
    fn find_binary_at_path_rejects_missing_explicit_path() {
        let cargo = std::env::var_os("CARGO").expect("cargo path");
        let cargo_path = PathBuf::from(cargo);
        let binary_name = cargo_path
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("cargo filename")
            .to_string();
        let missing_path = std::env::temp_dir()
            .join("ti-missing-explicit-command")
            .join(cargo_path.file_name().expect("cargo basename"));

        if let Some(parent) = missing_path.parent() {
            std::fs::create_dir_all(parent).expect("create temp parent");
        }
        if Path::new(&missing_path).exists() {
            std::fs::remove_file(&missing_path).expect("remove stale file");
        }

        assert!(find_binary_at_path(&missing_path, &binary_name).is_none());
    }

    #[test]
    fn resolve_npm_global_destination_ignores_unrelated_managed_dir_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        let package_root = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        let unrelated = temp.path().join("stale").join("http-server");
        std::fs::create_dir_all(unrelated.parent().expect("stale parent"))
            .expect("create stale parent");
        std::fs::write(&unrelated, "#!/bin/sh\nexit 0\n").expect("write stale binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&unrelated, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stale binary");
        }

        assert!(
            resolve_npm_global_destination(
                &binary_path,
                "http-server@14.1.1",
                "http-server",
                None,
                Some(&package_root),
            )
            .is_none()
        );
    }

    #[test]
    fn npm_global_package_dir_uses_package_name_without_version() {
        let package_dir =
            npm_global_package_dir(Path::new("/tmp/prefix"), "@scope/http-server@14.1.1");
        assert_eq!(
            package_dir,
            PathBuf::from("/tmp/prefix/lib/node_modules/@scope/http-server")
        );
    }

    #[test]
    fn resolve_package_bin_script_uses_manifest_instead_of_scanning_package_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        std::fs::create_dir_all(package_dir.join("dist")).expect("create package dir");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","bin":{"http-server":"dist/http-server.js"}}"#,
        )
        .expect("write manifest");

        let resolved =
            resolve_package_bin_script(&package_dir, "http-server@14.1.1", "http-server")
                .expect("resolved package bin path");
        assert_eq!(resolved, package_dir.join("dist").join("http-server.js"));
    }

    #[test]
    fn resolve_npm_global_destination_rejects_stale_binary_without_manifest_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        std::fs::create_dir_all(package_dir.join("bin")).expect("create package dir");
        let stale = package_dir.join("bin").join("http-server");
        std::fs::write(&stale, "#!/bin/sh\nexit 0\n").expect("write stale binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stale binary");
        }

        assert!(
            resolve_npm_global_destination(
                &binary_path,
                "http-server@14.1.1",
                "http-server",
                Some(&package_dir),
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn capture_installation_state_tracks_manifest_and_manifest_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        std::fs::create_dir_all(package_dir.join("bin")).expect("create package dir");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","bin":{"http-server":"bin/http-server"}}"#,
        )
        .expect("write manifest");
        std::fs::write(package_dir.join("bin").join("http-server"), "demo").expect("write bin");

        let captured = capture_installation_state(
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            None,
        );
        assert!(captured.contains_key(&package_dir.join("package.json")));
        assert!(captured.contains_key(&package_dir.join("bin").join("http-server")));
        assert!(
            captured
                .get(&package_dir.join("bin").join("http-server"))
                .is_some_and(|fingerprint| fingerprint
                    == &file_fingerprint(&package_dir.join("bin").join("http-server")))
        );
    }

    #[test]
    fn installation_result_accepts_unchanged_idempotent_binary_at_canonical_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        std::fs::create_dir_all(binary_path.parent().expect("binary parent"))
            .expect("create binary parent");
        std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod binary");
        }

        let preinstall_state = capture_installation_state(
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            None,
            None,
        );
        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            None,
            None,
        ));
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
