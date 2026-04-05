use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use omne_process_primitives::{
    HostRecipeRequest, command_path_exists, resolve_command_path, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{NodePackageManager, NpmGlobalPlanItem};

struct NpmGlobalRecipe {
    program: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    binary_path: PathBuf,
    fallback_package_dir: Option<PathBuf>,
    package_search_root: Option<PathBuf>,
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
        recipe.package_search_root.as_deref(),
        recipe.fallback_search_root.as_deref(),
    );
    run_host_recipe(
        &HostRecipeRequest::new(recipe.program.as_os_str(), &recipe.args).with_env(&recipe.env),
    )
    .map_err(OperationError::from_host_recipe)?;

    let destination = match resolve_npm_global_destination(
        &recipe.binary_path,
        &item.package_spec,
        &item.binary_name,
        recipe.fallback_package_dir.as_deref(),
        recipe.package_search_root.as_deref(),
        recipe.fallback_search_root.as_deref(),
    ) {
        Some(destination) => destination,
        None => create_windows_bun_global_launcher(
            item.manager,
            recipe.program.as_os_str(),
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
        &destination,
        &item.package_spec,
        &item.binary_name,
        recipe.fallback_package_dir.as_deref(),
        recipe.package_search_root.as_deref(),
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
        source_kind: Some(BootstrapSourceKind::NpmGlobal),
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
            let package_root = npm_global_package_root(&prefix_root, target_triple);
            let fallback_package_dir = package_dir_with_root(&package_root, &package);
            let binary_path = if target_triple.contains("windows") {
                prefix_root.join(global_binary_filename(binary_name, manager, target_triple))
            } else {
                prefix_root.join("bin").join(binary_name)
            };
            Ok(NpmGlobalRecipe {
                program: resolved_package_manager_program("npm"),
                args: vec![
                    OsString::from("install"),
                    OsString::from("--global"),
                    OsString::from("--prefix"),
                    prefix_root.as_os_str().to_os_string(),
                    OsString::from(package),
                ],
                env: vec![(
                    OsString::from("npm_config_prefix"),
                    prefix_root.as_os_str().to_os_string(),
                )],
                binary_path,
                fallback_package_dir: Some(fallback_package_dir),
                package_search_root: Some(package_root),
                fallback_search_root: None,
                source: "npm:npm".to_string(),
            })
        }
        NodePackageManager::Pnpm => {
            let program = resolved_package_manager_program("pnpm");
            let env = vec![
                (
                    OsString::from("PNPM_HOME"),
                    managed_dir.as_os_str().to_os_string(),
                ),
                (OsString::from("PATH"), prepend_path_env(managed_dir)?),
            ];
            let package_search_root = resolve_pnpm_global_package_root(&program, &env);
            let binary_path =
                managed_dir.join(global_binary_filename(binary_name, manager, target_triple));
            Ok(NpmGlobalRecipe {
                program,
                args: vec![
                    OsString::from("add"),
                    OsString::from("--global"),
                    OsString::from(package.clone()),
                ],
                env,
                binary_path,
                fallback_package_dir: package_search_root
                    .as_deref()
                    .map(|root| package_dir_with_root(root, &package)),
                package_search_root,
                fallback_search_root: None,
                source: "npm:pnpm".to_string(),
            })
        }
        NodePackageManager::Bun => {
            let layout = bun_install_layout(managed_dir)?;
            let global_dir = layout.global_dir;
            let binary_dir = layout.binary_dir;
            let package_root = global_dir.join("node_modules");
            let binary_path =
                binary_dir.join(global_binary_filename(binary_name, manager, target_triple));
            let fallback_package_dir = package_dir_with_root(&package_root, &package);
            Ok(NpmGlobalRecipe {
                program: resolved_package_manager_program("bun"),
                args: vec![
                    OsString::from("add"),
                    OsString::from("--global"),
                    OsString::from(package),
                ],
                env: vec![
                    (
                        OsString::from("BUN_INSTALL_GLOBAL_DIR"),
                        global_dir.as_os_str().to_os_string(),
                    ),
                    (
                        OsString::from("BUN_INSTALL_BIN"),
                        binary_dir.as_os_str().to_os_string(),
                    ),
                    (OsString::from("PATH"), prepend_path_env(&binary_dir)?),
                ],
                binary_path,
                fallback_package_dir: Some(fallback_package_dir),
                package_search_root: Some(package_root),
                fallback_search_root: Some(global_dir.join("node_modules").join(".bin")),
                source: "npm:bun".to_string(),
            })
        }
    }
}

fn resolved_package_manager_program(command: &str) -> OsString {
    resolve_command_path(command)
        .map(|path| path.into_os_string())
        .unwrap_or_else(|| OsString::from(command))
}

fn resolve_pnpm_global_package_root(
    program: &OsStr,
    env: &[(OsString, OsString)],
) -> Option<PathBuf> {
    let args = [OsString::from("root"), OsString::from("--global")];
    let output = run_host_recipe(&HostRecipeRequest::new(program, &args).with_env(env)).ok()?;
    parse_pnpm_root_stdout(&output.output.stdout)
}

fn parse_pnpm_root_stdout(stdout: &[u8]) -> Option<PathBuf> {
    let stdout = std::str::from_utf8(stdout).ok()?;
    let root = stdout.lines().find(|line| !line.trim().is_empty())?.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
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

fn npm_global_package_root(prefix_root: &Path, target_triple: &str) -> PathBuf {
    if target_triple.contains("windows") {
        return prefix_root.join("node_modules");
    }
    prefix_root.join("lib").join("node_modules")
}

struct BunInstallLayout {
    global_dir: PathBuf,
    binary_dir: PathBuf,
}

fn bun_install_layout(managed_dir: &Path) -> OperationResult<BunInstallLayout> {
    let install_root = if managed_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == "bin")
    {
        managed_dir
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| OperationError::install("cannot determine bun install root"))?
    } else {
        managed_dir.to_path_buf()
    };
    let binary_dir = if managed_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == "bin")
    {
        managed_dir.to_path_buf()
    } else {
        managed_dir.join("bin")
    };
    Ok(BunInstallLayout {
        global_dir: install_root.join("install").join("global"),
        binary_dir,
    })
}

fn resolve_npm_global_destination(
    binary_path: &Path,
    _package: &str,
    binary_name: &str,
    _fallback_package_dir: Option<&Path>,
    _package_search_root: Option<&Path>,
    _fallback_search_root: Option<&Path>,
) -> Option<PathBuf> {
    find_binary_at_path(binary_path, binary_name)
}

fn capture_installation_state(
    binary_path: &Path,
    package: &str,
    binary_name: &str,
    fallback_package_dir: Option<&Path>,
    package_search_root: Option<&Path>,
    fallback_search_root: Option<&Path>,
) -> HashMap<PathBuf, Option<FileFingerprint>> {
    let mut paths = candidate_binary_paths(binary_path, binary_name);
    for package_dir in
        resolve_installed_package_dirs(package, fallback_package_dir, package_search_root)
    {
        let manifest_path = package_dir.join("package.json");
        paths.push(manifest_path.clone());
        if let Some(destination) = resolve_package_bin_script(&package_dir, package, binary_name) {
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
    destination: &Path,
    package: &str,
    binary_name: &str,
    fallback_package_dir: Option<&Path>,
    package_search_root: Option<&Path>,
    fallback_search_root: Option<&Path>,
) -> bool {
    if path_changed(preinstall_state, destination) {
        return true;
    }

    destination_preexisted(preinstall_state, destination)
        && managed_package_matches_request(
            package,
            binary_name,
            fallback_package_dir,
            package_search_root,
            fallback_search_root,
        )
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

fn managed_package_matches_request(
    package: &str,
    binary_name: &str,
    fallback_package_dir: Option<&Path>,
    package_search_root: Option<&Path>,
    fallback_search_root: Option<&Path>,
) -> bool {
    for package_dir in
        resolve_installed_package_dirs(package, fallback_package_dir, package_search_root)
    {
        let manifest_path = package_dir.join("package.json");
        if !manifest_satisfies_package_request(&manifest_path, package, binary_name) {
            continue;
        }

        if let Some(path) = resolve_package_bin_script(&package_dir, package, binary_name)
            && command_path_exists(&path)
        {
            return true;
        }
    }
    let _ = fallback_search_root;
    false
}

fn resolve_installed_package_dirs(
    package: &str,
    direct_package_dir: Option<&Path>,
    package_search_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut package_dirs = Vec::new();
    if let Some(path) =
        direct_package_dir.filter(|path| path_has_no_symlink_components(path, PathKind::Directory))
    {
        let manifest_path = path.join("package.json");
        if path_has_no_symlink_components(&manifest_path, PathKind::File) {
            package_dirs.push(path.to_path_buf());
        }
    }
    if let Some(root) =
        package_search_root.filter(|path| path_has_no_symlink_components(path, PathKind::Directory))
    {
        package_dirs.extend(find_package_dirs_under_root(
            root,
            package_request_name_constraint(package),
        ));
    }
    package_dirs.sort();
    package_dirs.dedup();
    package_dirs
}

#[derive(Clone, Copy)]
enum PathKind {
    Directory,
    File,
}

fn path_has_no_symlink_components(path: &Path, kind: PathKind) -> bool {
    let mut current = PathBuf::new();
    let mut saw_component = false;
    for component in path.components() {
        current.push(component.as_os_str());
        let Ok(metadata) = std::fs::symlink_metadata(&current) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        saw_component = true;
    }
    if !saw_component {
        return false;
    }
    std::fs::symlink_metadata(path).is_ok_and(|metadata| match kind {
        PathKind::Directory => metadata.is_dir(),
        PathKind::File => metadata.is_file(),
    })
}

fn manifest_path_is_searchable(path: &Path) -> bool {
    path_has_no_symlink_components(path, PathKind::File)
}

fn find_package_dirs_under_root(root: &Path, package_name: Option<&str>) -> Vec<PathBuf> {
    if !path_has_no_symlink_components(root, PathKind::Directory) {
        return Vec::new();
    }
    let mut stack = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(dir) = stack.pop() {
        let manifest_path = dir.join("package.json");
        if manifest_path_is_searchable(&manifest_path) {
            let matches_name = std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|manifest| package_name_from_manifest(&manifest))
                .is_some_and(|name| package_name.is_none_or(|expected| name == expected));
            if matches_name {
                matches.push(dir.clone());
            }
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            if entry_is_plain_directory(&entry) {
                stack.push(entry.path());
            }
        }
    }
    matches.sort();
    matches
}

fn find_matching_binary_paths_under_dir(root: &Path, binary_name: &str) -> Vec<PathBuf> {
    if !path_has_no_symlink_components(root, PathKind::Directory) {
        return Vec::new();
    }
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
            if entry_is_plain_directory(&entry) {
                stack.push(path);
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if (file_type.is_file() || (file_type.is_symlink() && path.is_file()))
                && path
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

fn manifest_satisfies_package_request(
    manifest_path: &Path,
    package: &str,
    binary_name: &str,
) -> bool {
    let Ok(manifest) = std::fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest) else {
        return false;
    };
    let Some(name) = manifest.get("name").and_then(|value| value.as_str()) else {
        return false;
    };
    let request = npm_package_request(package);
    if let Some(expected_name) = request.package_name()
        && name != expected_name
    {
        return false;
    }

    match request {
        NpmPackageRequest::ExactVersion { version, .. } => manifest
            .get("version")
            .and_then(|value| value.as_str())
            .is_some_and(|installed| installed == version),
        NpmPackageRequest::Named { .. } => true,
        NpmPackageRequest::ExplicitSource { .. } => {
            package_bin_relative_path(&manifest, package, binary_name).is_some()
        }
    }
}

#[cfg(test)]
fn npm_global_package_dir(prefix_root: &Path, package: &str, target_triple: &str) -> PathBuf {
    package_dir_with_root(
        &npm_global_package_root(prefix_root, target_triple),
        package,
    )
}

fn package_dir_with_root(root: &Path, package: &str) -> PathBuf {
    let mut package_dir = root.to_path_buf();
    for segment in npm_package_name(package).split('/') {
        package_dir.push(segment);
    }
    package_dir
}

#[cfg(windows)]
fn create_windows_bun_global_launcher(
    manager: NodePackageManager,
    bun_program: &OsStr,
    managed_dir: &Path,
    package: &str,
    binary_name: &str,
) -> OperationResult<Option<PathBuf>> {
    if !matches!(manager, NodePackageManager::Bun) {
        return Ok(None);
    }

    let layout = bun_install_layout(managed_dir)?;
    let global_dir = layout.global_dir;
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

    let launcher_path = layout.binary_dir.join(format!("{binary_name}.cmd"));
    if let Some(parent) = launcher_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| OperationError::install(err.to_string()))?;
    }
    let launcher_body = format!(
        "@echo off\r\n\"{}\" \"{}\" %*\r\n",
        bun_program.to_string_lossy(),
        script_path.display()
    );
    std::fs::write(&launcher_path, launcher_body)
        .map_err(|err| OperationError::install(err.to_string()))?;
    Ok(Some(launcher_path))
}

#[cfg(not(windows))]
fn create_windows_bun_global_launcher(
    _manager: NodePackageManager,
    _bun_program: &OsStr,
    _managed_dir: &Path,
    _package: &str,
    _binary_name: &str,
) -> OperationResult<Option<PathBuf>> {
    Ok(None)
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
    find_package_dirs_under_root(global_dir, Some(npm_package_name(package)))
        .into_iter()
        .next()
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

fn package_name_from_manifest(manifest: &str) -> Option<String> {
    let manifest: serde_json::Value = serde_json::from_str(manifest).ok()?;
    manifest
        .get("name")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NpmPackageRequest<'a> {
    ExactVersion {
        package_name: &'a str,
        version: &'a str,
    },
    Named {
        package_name: &'a str,
    },
    ExplicitSource {
        package_name: Option<&'a str>,
    },
}

impl NpmPackageRequest<'_> {
    fn package_name(&self) -> Option<&str> {
        match self {
            Self::ExactVersion { package_name, .. } | Self::Named { package_name } => {
                Some(package_name)
            }
            Self::ExplicitSource { package_name } => *package_name,
        }
    }
}

fn npm_package_request(package: &str) -> NpmPackageRequest<'_> {
    if node_package_spec_uses_explicit_source(package) {
        return NpmPackageRequest::ExplicitSource {
            package_name: npm_package_name_from_explicit_source(package),
        };
    }

    let package_name = npm_package_name(package);
    match npm_package_version(package) {
        Some(version) if npm_package_version_is_exact(version) => NpmPackageRequest::ExactVersion {
            package_name,
            version,
        },
        Some(_) | None => NpmPackageRequest::Named { package_name },
    }
}

fn package_request_name_constraint(package: &str) -> Option<&str> {
    match npm_package_request(package) {
        NpmPackageRequest::ExactVersion { package_name, .. }
        | NpmPackageRequest::Named { package_name } => Some(package_name),
        NpmPackageRequest::ExplicitSource { package_name } => package_name,
    }
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

fn npm_package_version(package: &str) -> Option<&str> {
    let package = package.trim();
    let (name_or_scope, version) = package.rsplit_once('@')?;
    if package.starts_with('@') && !name_or_scope.contains('/') {
        return None;
    }
    (!version.is_empty()).then_some(version)
}

fn npm_package_version_is_exact(version: &str) -> bool {
    let version = version.trim();
    if version.is_empty()
        || version.eq_ignore_ascii_case("latest")
        || version.contains([' ', '^', '~', '<', '>', '=', '*', '|'])
        || version.ends_with(".x")
        || version.ends_with(".*")
    {
        return false;
    }

    let version = version.strip_prefix('v').unwrap_or(version);
    version.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'))
}

fn npm_package_name_from_explicit_source(package: &str) -> Option<&str> {
    let package = package.trim();
    if let Some((_, source)) = package.split_once("@npm:") {
        return Some(npm_package_name(source));
    }
    package.strip_prefix("npm:").map(npm_package_name)
}

fn node_package_spec_uses_explicit_source(package: &str) -> bool {
    if node_package_spec_is_local_path(package) {
        return true;
    }
    [
        "file:",
        "git:",
        "git+",
        "http://",
        "https://",
        "github:",
        "workspace:",
        "link:",
        "npm:",
    ]
    .iter()
    .any(|prefix| package.starts_with(prefix))
        || package.contains("@npm:")
}

fn node_package_spec_is_local_path(package: &str) -> bool {
    let package = package.trim();
    if package.starts_with("git+")
        || package.starts_with("git:")
        || package.starts_with("github:")
        || package.starts_with("workspace:")
        || package.starts_with("link:")
        || package.starts_with("npm:")
        || package.starts_with("file:")
        || package.contains("@npm:")
        || package.contains("://")
    {
        return false;
    }
    package == "."
        || package == ".."
        || package.starts_with("./")
        || package.starts_with(".\\")
        || package.starts_with("../")
        || package.starts_with("..\\")
        || package.starts_with('/')
        || package.starts_with('\\')
        || looks_like_windows_drive_path(package)
        || (package.contains(['/', '\\']) && !package.starts_with('@'))
}

fn looks_like_windows_drive_path(package: &str) -> bool {
    let bytes = package.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn entry_is_plain_directory(entry: &std::fs::DirEntry) -> bool {
    entry
        .file_type()
        .is_ok_and(|file_type| file_type.is_dir() && !file_type.is_symlink())
}

fn prepend_path_env(path: &Path) -> OperationResult<OsString> {
    let mut entries = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(entries)
        .map_err(|err| OperationError::install(format!("cannot compose PATH: {err}")))?;
    Ok(joined)
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
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::{
        NpmPackageRequest, build_npm_global_recipe, capture_installation_state, file_fingerprint,
        find_binary_at_path, find_matching_binary_paths_under_dir, find_package_dirs_under_root,
        installation_result_is_acceptable, npm_global_package_dir, npm_package_request,
        package_bin_relative_path, parse_pnpm_root_stdout, path_has_no_symlink_components,
        resolve_installed_package_dirs, resolve_npm_global_destination, resolve_package_bin_script,
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

        assert!(recipe.env.iter().any(|(name, value)| {
            name == OsStr::new("PNPM_HOME") && value == managed_dir.as_os_str()
        }));
        let path = recipe
            .env
            .iter()
            .find(|(name, _)| name == OsStr::new("PATH"))
            .map(|(_, value)| value.as_os_str())
            .expect("PATH env");
        let first = std::env::split_paths(path)
            .next()
            .expect("first PATH entry");
        assert_eq!(first, managed_dir);
    }

    #[test]
    fn parse_pnpm_root_stdout_uses_first_non_empty_line() {
        assert_eq!(
            parse_pnpm_root_stdout(b"\n/tmp/pnpm-global/node_modules\nignored\n"),
            Some(PathBuf::from("/tmp/pnpm-global/node_modules"))
        );
    }

    #[test]
    fn parse_pnpm_root_stdout_rejects_empty_or_non_utf8_output() {
        assert_eq!(parse_pnpm_root_stdout(b"\n \n"), None);
        assert_eq!(parse_pnpm_root_stdout(&[0xff, 0xfe]), None);
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
            name == OsStr::new("BUN_INSTALL_GLOBAL_DIR") && value == expected_global_dir.as_os_str()
        }));
        let expected_binary_dir = managed_dir.join("bin");
        assert!(recipe.env.iter().any(|(name, value)| {
            name == OsStr::new("BUN_INSTALL_BIN") && value == expected_binary_dir.as_os_str()
        }));
        assert_eq!(
            recipe.binary_path.parent(),
            Some(expected_binary_dir.as_path())
        );
        let path = recipe
            .env
            .iter()
            .find(|(name, _)| name == OsStr::new("PATH"))
            .map(|(_, value)| value.as_os_str())
            .expect("PATH env");
        let first = std::env::split_paths(path)
            .next()
            .expect("first PATH entry");
        assert_eq!(first, expected_binary_dir);
    }

    #[test]
    fn bun_recipe_reuses_managed_bin_dir_when_managed_dir_already_is_bin() {
        let managed_dir = std::env::temp_dir().join("ti-bun-root").join("bin");
        let recipe = build_npm_global_recipe(
            NodePackageManager::Bun,
            "http-server@14.1.1".to_string(),
            "http-server",
            host_target_triple(),
            &managed_dir,
        )
        .expect("build bun recipe");

        let expected_global_dir = managed_dir
            .parent()
            .expect("managed parent")
            .join("install")
            .join("global");
        assert!(recipe.env.iter().any(|(name, value)| {
            name == OsStr::new("BUN_INSTALL_GLOBAL_DIR") && value == expected_global_dir.as_os_str()
        }));
        assert!(recipe.env.iter().any(|(name, value)| {
            name == OsStr::new("BUN_INSTALL_BIN") && value == managed_dir.as_os_str()
        }));
        assert_eq!(recipe.binary_path.parent(), Some(managed_dir.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn npm_recipe_preserves_non_utf8_prefix_arg_and_env() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let managed_dir = PathBuf::from(OsString::from_vec(b"/tmp/npm-managed-\xff".to_vec()));
        let recipe = build_npm_global_recipe(
            NodePackageManager::Npm,
            "http-server".to_string(),
            "http-server",
            host_target_triple(),
            &managed_dir,
        )
        .expect("build npm recipe");

        let expected_prefix = if managed_dir
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value == "bin")
        {
            managed_dir
                .parent()
                .expect("managed parent")
                .as_os_str()
                .as_bytes()
                .to_vec()
        } else {
            managed_dir.as_os_str().as_bytes().to_vec()
        };

        assert_eq!(recipe.args[3].as_bytes(), expected_prefix);
        assert!(recipe.env.iter().any(|(name, value)| {
            name == OsStr::new("npm_config_prefix")
                && value.as_bytes() == expected_prefix.as_slice()
        }));
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
                None,
                Some(&package_root),
            )
            .is_none()
        );
    }

    #[test]
    fn npm_global_package_dir_uses_package_name_without_version() {
        let package_dir = npm_global_package_dir(
            Path::new("/tmp/prefix"),
            "@scope/http-server@14.1.1",
            "x86_64-unknown-linux-gnu",
        );
        assert_eq!(
            package_dir,
            PathBuf::from("/tmp/prefix/lib/node_modules/@scope/http-server")
        );
    }

    #[test]
    fn npm_global_package_dir_uses_windows_node_modules_root() {
        let package_dir = npm_global_package_dir(
            Path::new(r"C:\prefix"),
            "@scope/http-server@14.1.1",
            "x86_64-pc-windows-msvc",
        );
        assert_eq!(
            package_dir,
            PathBuf::from(r"C:\prefix/node_modules/@scope/http-server")
        );
    }

    #[test]
    fn npm_recipe_uses_windows_metadata_layout_for_idempotency_checks() {
        let managed_dir = Path::new(r"C:\managed");
        let recipe = build_npm_global_recipe(
            NodePackageManager::Npm,
            "http-server@14.1.1".to_string(),
            "http-server",
            "x86_64-pc-windows-msvc",
            managed_dir,
        )
        .expect("build npm recipe");

        assert_eq!(
            recipe.binary_path,
            PathBuf::from(r"C:\managed/http-server.cmd")
        );
        assert_eq!(
            recipe.package_search_root,
            Some(PathBuf::from(r"C:\managed/node_modules"))
        );
        assert_eq!(
            recipe.fallback_package_dir,
            Some(PathBuf::from(r"C:\managed/node_modules/http-server"))
        );
    }

    #[test]
    fn npm_package_request_only_treats_exact_versions_as_exact() {
        assert_eq!(
            npm_package_request("http-server@14.1.1"),
            NpmPackageRequest::ExactVersion {
                package_name: "http-server",
                version: "14.1.1",
            }
        );
        assert_eq!(
            npm_package_request("http-server@latest"),
            NpmPackageRequest::Named {
                package_name: "http-server",
            }
        );
        assert_eq!(
            npm_package_request("http-server@^14"),
            NpmPackageRequest::Named {
                package_name: "http-server",
            }
        );
        assert_eq!(
            npm_package_request("file:../packages/http-server"),
            NpmPackageRequest::ExplicitSource { package_name: None }
        );
        assert_eq!(
            npm_package_request("../packages/http-server"),
            NpmPackageRequest::ExplicitSource { package_name: None }
        );
        assert_eq!(
            npm_package_request("alias@npm:http-server@14.1.1"),
            NpmPackageRequest::ExplicitSource {
                package_name: Some("http-server"),
            }
        );
    }

    #[test]
    fn installation_result_accepts_noop_for_tag_spec_with_matching_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        let binary_path = temp.path().join("bin").join("http-server");
        write_package_with_binary(
            &package_dir,
            &binary_path,
            "http-server",
            "14.1.1",
            "bin/http-server",
        );
        let preinstall_state = capture_installation_state(
            &binary_path,
            "http-server@latest",
            "http-server",
            Some(&package_dir),
            None,
            None,
        );

        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@latest",
            "http-server",
            Some(&package_dir),
            None,
            None,
        ));
    }

    #[test]
    fn installation_result_accepts_noop_for_explicit_source_with_matching_package_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_dir = temp.path().join("global").join("pkg");
        let binary_path = temp.path().join("bin").join("demo");
        write_package_with_binary(&package_dir, &binary_path, "demo", "1.0.0", "bin/demo");
        let preinstall_state = capture_installation_state(
            &binary_path,
            "file:../packages/demo",
            "demo",
            Some(&package_dir),
            None,
            None,
        );

        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "file:../packages/demo",
            "demo",
            Some(&package_dir),
            None,
            None,
        ));
    }

    #[test]
    fn installation_result_accepts_noop_for_alias_spec_with_matching_source_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_root = temp.path().join("lib").join("node_modules");
        let package_dir = package_root.join("alias");
        let binary_path = temp.path().join("bin").join("http-server");
        write_package_with_binary(
            &package_dir,
            &binary_path,
            "http-server",
            "14.1.1",
            "bin/http-server",
        );
        let preinstall_state = capture_installation_state(
            &binary_path,
            "alias@npm:http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            Some(&package_root),
            None,
        );

        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "alias@npm:http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            Some(&package_root),
            None,
        ));
    }

    #[test]
    fn installation_result_accepts_noop_for_local_path_source_with_matching_package_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_root = temp.path().join("global");
        let package_dir = package_root.join("pkg");
        let binary_path = temp.path().join("bin").join("demo");
        write_package_with_binary(&package_dir, &binary_path, "demo", "1.0.0", "bin/demo");
        let preinstall_state = capture_installation_state(
            &binary_path,
            "../packages/demo",
            "demo",
            None,
            Some(&package_root),
            None,
        );

        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "../packages/demo",
            "demo",
            None,
            Some(&package_root),
            None,
        ));
    }

    #[test]
    fn installation_result_accepts_noop_for_pnpm_global_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_root = temp.path().join("global").join("5").join("node_modules");
        let package_dir = package_root.join("http-server");
        let binary_path = temp.path().join("http-server");
        write_package_with_binary(
            &package_dir,
            &binary_path,
            "http-server",
            "14.1.1",
            "bin/http-server",
        );
        let preinstall_state = capture_installation_state(
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            None,
            Some(temp.path().join("global").as_path()),
            None,
        );

        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            None,
            Some(temp.path().join("global").as_path()),
            None,
        ));
    }

    #[test]
    fn resolve_installed_package_dirs_finds_versioned_pnpm_global_package_from_reported_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let package_root = temp.path().join("global").join("5").join("node_modules");
        let package_dir = package_root.join("http-server");
        write_package_with_binary(
            &package_dir,
            &temp.path().join("http-server"),
            "http-server",
            "14.1.1",
            "bin/http-server",
        );

        assert_eq!(
            resolve_installed_package_dirs("http-server@14.1.1", None, Some(&package_root)),
            vec![package_dir]
        );
    }

    #[test]
    fn installation_result_rejects_orphan_binary_without_package_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("demo");
        write_binary(&binary_path);
        let preinstall_state = capture_installation_state(
            &binary_path,
            "file:../packages/demo",
            "demo",
            None,
            None,
            None,
        );

        assert!(!installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "file:../packages/demo",
            "demo",
            None,
            None,
            None,
        ));
    }

    #[test]
    fn resolve_npm_global_destination_skips_invalid_direct_package_dir_and_scans_search_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let direct_package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        let search_root = temp.path().join("global");
        let scanned_package_dir = search_root.join("store").join("http-server");
        let binary_path = temp.path().join("bin").join("http-server");

        std::fs::create_dir_all(&direct_package_dir).expect("create direct package dir");
        std::fs::write(direct_package_dir.join("package.json"), "{not-json")
            .expect("write broken manifest");
        write_package_with_binary(
            &scanned_package_dir,
            &binary_path,
            "http-server",
            "14.1.1",
            "bin/http-server",
        );

        assert_eq!(
            resolve_npm_global_destination(
                &binary_path,
                "http-server@latest",
                "http-server",
                Some(&direct_package_dir),
                Some(&search_root),
                None,
            ),
            Some(binary_path)
        );
    }

    #[test]
    fn find_package_dirs_under_root_skips_bad_manifest_and_keeps_searching() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let bad = root.join("bad");
        let good = root.join("good");
        std::fs::create_dir_all(&bad).expect("create bad dir");
        std::fs::create_dir_all(&good).expect("create good dir");
        std::fs::write(bad.join("package.json"), "{not-json").expect("write broken manifest");
        std::fs::write(
            good.join("package.json"),
            r#"{"name":"http-server","version":"14.1.1"}"#,
        )
        .expect("write good manifest");

        assert_eq!(
            find_package_dirs_under_root(root, Some("http-server")),
            vec![good]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_installed_package_dirs_rejects_symlinked_direct_package_dir() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside_dir = temp.path().join("outside");
        std::fs::create_dir_all(&outside_dir).expect("create outside dir");
        std::fs::write(
            outside_dir.join("package.json"),
            r#"{"name":"http-server","version":"14.1.1","bin":"bin/http-server"}"#,
        )
        .expect("write outside manifest");

        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        std::fs::create_dir_all(package_dir.parent().expect("package parent"))
            .expect("create package parent");
        symlink(&outside_dir, &package_dir).expect("create symlink");

        let package_dirs =
            super::resolve_installed_package_dirs("http-server@14.1.1", Some(&package_dir), None);
        assert!(package_dirs.is_empty());
        assert!(!path_has_no_symlink_components(
            &package_dir.join("package.json"),
            super::PathKind::File
        ));
    }

    fn write_package_with_binary(
        package_dir: &Path,
        binary_path: &Path,
        package_name: &str,
        version: &str,
        relative_binary_path: &str,
    ) {
        std::fs::create_dir_all(package_dir).expect("create package dir");
        std::fs::write(
            package_dir.join("package.json"),
            format!(
                r#"{{"name":"{package_name}","version":"{version}","bin":"{relative_binary_path}"}}"#
            ),
        )
        .expect("write manifest");
        write_binary(&package_dir.join(relative_binary_path));
        write_binary(binary_path);
    }

    fn write_binary(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create binary parent");
        }
        std::fs::write(path, "#!/bin/sh\nexit 0\n").expect("write binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod binary");
        }
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
                None,
            )
            .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_matching_binary_paths_under_dir_skips_directory_symlink_loops() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let search_root = temp.path().join("global");
        let real_dir = search_root.join("node_modules").join(".bin");
        std::fs::create_dir_all(&real_dir).expect("create real dir");
        std::fs::write(real_dir.join("http-server"), "#!/bin/sh\nexit 0\n").expect("write binary");
        symlink(&search_root, search_root.join("loop")).expect("create loop symlink");

        let matches = find_matching_binary_paths_under_dir(&search_root, "http-server");
        assert_eq!(matches, vec![real_dir.join("http-server")]);
    }

    #[cfg(unix)]
    #[test]
    fn find_matching_binary_paths_under_dir_does_not_traverse_external_directory_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let search_root = temp.path().join("global");
        let external_root = temp.path().join("external");
        let real_dir = search_root.join("node_modules").join(".bin");
        let external_dir = external_root.join(".bin");
        std::fs::create_dir_all(&real_dir).expect("create real dir");
        std::fs::create_dir_all(&external_dir).expect("create external dir");
        std::fs::write(real_dir.join("http-server"), "#!/bin/sh\nexit 0\n").expect("write binary");
        std::fs::write(external_dir.join("http-server"), "#!/bin/sh\nexit 0\n")
            .expect("write external binary");
        symlink(&external_root, search_root.join("escape")).expect("create escape symlink");

        let matches = find_matching_binary_paths_under_dir(&search_root, "http-server");
        assert_eq!(matches, vec![real_dir.join("http-server")]);
    }

    #[cfg(unix)]
    #[test]
    fn find_package_dirs_under_root_skips_directory_symlink_loops() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let search_root = temp.path().join("global");
        let package_dir = search_root.join("node_modules").join("http-server");
        std::fs::create_dir_all(&package_dir).expect("create package dir");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","version":"14.1.1"}"#,
        )
        .expect("write package.json");
        symlink(&search_root, search_root.join("loop")).expect("create loop symlink");

        let resolved = find_package_dirs_under_root(&search_root, Some("http-server"));
        assert_eq!(resolved, vec![package_dir]);
    }

    #[cfg(unix)]
    #[test]
    fn find_package_dirs_under_root_does_not_traverse_external_directory_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let search_root = temp.path().join("global");
        let package_dir = search_root.join("node_modules").join("http-server");
        let external_root = temp.path().join("external");
        let external_package_dir = external_root.join("node_modules").join("http-server");
        std::fs::create_dir_all(&package_dir).expect("create package dir");
        std::fs::create_dir_all(&external_package_dir).expect("create external package dir");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","version":"14.1.1"}"#,
        )
        .expect("write package.json");
        std::fs::write(
            external_package_dir.join("package.json"),
            r#"{"name":"http-server","version":"99.0.0"}"#,
        )
        .expect("write external package.json");
        symlink(&external_root, search_root.join("escape")).expect("create escape symlink");

        let resolved = find_package_dirs_under_root(&search_root, Some("http-server"));
        assert_eq!(resolved, vec![package_dir]);
    }

    #[test]
    fn find_package_dirs_under_root_skips_unreadable_or_invalid_manifests() {
        let temp = tempfile::tempdir().expect("tempdir");
        let search_root = temp.path().join("global");
        let broken_dir = search_root.join("node_modules").join("broken");
        let package_dir = search_root.join("node_modules").join("http-server");
        std::fs::create_dir_all(&broken_dir).expect("create broken dir");
        std::fs::create_dir_all(&package_dir).expect("create package dir");
        std::fs::write(broken_dir.join("package.json"), "{not-json").expect("write broken json");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","version":"14.1.1"}"#,
        )
        .expect("write package manifest");

        let resolved = find_package_dirs_under_root(&search_root, Some("http-server"));
        assert_eq!(resolved, vec![package_dir]);
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
            r#"{"name":"http-server","version":"14.1.1","bin":{"http-server":"bin/http-server"}}"#,
        )
        .expect("write manifest");
        std::fs::write(package_dir.join("bin").join("http-server"), "demo").expect("write bin");

        let captured = capture_installation_state(
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            None,
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
    fn installation_result_accepts_unchanged_idempotent_binary_when_manifest_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        std::fs::create_dir_all(binary_path.parent().expect("binary parent"))
            .expect("create binary parent");
        std::fs::create_dir_all(package_dir.join("bin")).expect("create package dir");
        std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write binary");
        std::fs::write(
            package_dir.join("bin").join("http-server"),
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write package bin");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","version":"14.1.1","bin":{"http-server":"bin/http-server"}}"#,
        )
        .expect("write manifest");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod binary");
            std::fs::set_permissions(
                package_dir.join("bin").join("http-server"),
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("chmod package bin");
        }

        let preinstall_state = capture_installation_state(
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            None,
            None,
        );
        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            None,
            None,
        ));
    }

    #[test]
    fn installation_result_accepts_npm_source_spec_when_manifest_matches_real_package() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        let package_root = temp.path().join("lib").join("node_modules");
        let package_dir = package_root.join("@scope").join("http-server");
        std::fs::create_dir_all(binary_path.parent().expect("binary parent"))
            .expect("create binary parent");
        std::fs::create_dir_all(package_dir.join("bin")).expect("create package dir");
        std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write binary");
        std::fs::write(
            package_dir.join("bin").join("http-server"),
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write package bin");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"@scope/http-server","version":"14.1.1","bin":{"http-server":"bin/http-server"}}"#,
        )
        .expect("write manifest");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod binary");
            std::fs::set_permissions(
                package_dir.join("bin").join("http-server"),
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("chmod package bin");
        }

        let preinstall_state = capture_installation_state(
            &binary_path,
            "npm:@scope/http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            Some(&package_root),
            None,
        );
        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "npm:@scope/http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            Some(&package_root),
            None,
        ));
    }

    #[test]
    fn installation_result_accepts_github_source_spec_when_manifest_exposes_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("repo-tool");
        let package_root = temp.path().join("lib").join("node_modules");
        let package_dir = package_root.join("repo-tool-package");
        std::fs::create_dir_all(binary_path.parent().expect("binary parent"))
            .expect("create binary parent");
        std::fs::create_dir_all(package_dir.join("dist")).expect("create package dir");
        std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write binary");
        std::fs::write(
            package_dir.join("dist").join("repo-tool.js"),
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write package bin");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"custom-package-name","bin":{"repo-tool":"dist/repo-tool.js"}}"#,
        )
        .expect("write manifest");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod binary");
            std::fs::set_permissions(
                package_dir.join("dist").join("repo-tool.js"),
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("chmod package bin");
        }

        let preinstall_state = capture_installation_state(
            &binary_path,
            "github:owner/repo-tool#main",
            "repo-tool",
            None,
            Some(&package_root),
            None,
        );
        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "github:owner/repo-tool#main",
            "repo-tool",
            None,
            Some(&package_root),
            None,
        ));
    }

    #[test]
    fn installation_result_rejects_unchanged_orphan_binary() {
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
            None,
        );
        assert!(!installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            None,
            None,
            None,
        ));
    }

    #[test]
    fn installation_result_rejects_unchanged_binary_when_manifest_version_mismatches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bin").join("http-server");
        let package_dir = temp
            .path()
            .join("lib")
            .join("node_modules")
            .join("http-server");
        std::fs::create_dir_all(binary_path.parent().expect("binary parent"))
            .expect("create binary parent");
        std::fs::create_dir_all(package_dir.join("bin")).expect("create package dir");
        std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write binary");
        std::fs::write(
            package_dir.join("bin").join("http-server"),
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write package bin");
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","version":"14.1.0","bin":{"http-server":"bin/http-server"}}"#,
        )
        .expect("write manifest");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod binary");
            std::fs::set_permissions(
                package_dir.join("bin").join("http-server"),
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("chmod package bin");
        }

        let preinstall_state = capture_installation_state(
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            Some(&package_dir),
            None,
            None,
        );
        assert!(!installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
            "http-server",
            Some(&package_dir),
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
