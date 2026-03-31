use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use omne_process_primitives::{
    HostRecipeRequest, command_path_exists, resolve_command_path, run_host_recipe,
};

use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};
use crate::error::{OperationError, OperationResult};
use crate::plan_items::{NodePackageManager, NpmGlobalPlanItem};

struct NpmGlobalRecipe {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    binary_path: PathBuf,
    package_dir: Option<PathBuf>,
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
    let preinstall_state =
        capture_installation_state(&recipe.binary_path, recipe.package_dir.as_deref());
    let recipe_args = recipe.args.iter().map(OsString::from).collect::<Vec<_>>();
    let recipe_env = recipe
        .env
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect::<Vec<_>>();
    run_host_recipe(
        &HostRecipeRequest::new(recipe.program.as_ref(), &recipe_args).with_env(&recipe_env),
    )
    .map_err(OperationError::from_host_recipe)?;

    let destination = recipe.binary_path.clone();
    if !installation_result_is_acceptable(
        &preinstall_state,
        &destination,
        &item.package_spec,
        recipe.package_dir.as_deref(),
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
            let package_dir = npm_global_package_dir(&prefix_root, &package);
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
                package_dir: Some(package_dir),
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
                package_dir: None,
                source: "npm:pnpm".to_string(),
            })
        }
        NodePackageManager::Bun => {
            let global_dir = managed_dir.join("install").join("global");
            let binary_dir = managed_dir.join("bin");
            let binary_path =
                binary_dir.join(global_binary_filename(binary_name, manager, target_triple));
            let package_dir = package_dir_with_root(&global_dir.join("node_modules"), &package);
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
                package_dir: Some(package_dir),
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

fn capture_installation_state(
    binary_path: &Path,
    package_dir: Option<&Path>,
) -> HashMap<PathBuf, Option<FileFingerprint>> {
    let mut paths = vec![binary_path.to_path_buf()];
    if let Some(package_dir) = resolve_expected_package_dir(package_dir) {
        let manifest_path = package_dir.join("package.json");
        paths.push(manifest_path.clone());
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
    package_dir: Option<&Path>,
) -> bool {
    if path_changed(preinstall_state, destination) {
        return true;
    }

    destination_preexisted(preinstall_state, destination)
        && managed_package_matches_request(package, package_dir)
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

fn managed_package_matches_request(package: &str, package_dir: Option<&Path>) -> bool {
    let Some(package_dir) = resolve_expected_package_dir(package_dir) else {
        return false;
    };
    let manifest_path = package_dir.join("package.json");
    manifest_satisfies_package_request(&manifest_path, package)
}

fn resolve_expected_package_dir(package_dir: Option<&Path>) -> Option<PathBuf> {
    package_dir
        .filter(|path| path.join("package.json").is_file())
        .map(Path::to_path_buf)
}

fn manifest_satisfies_package_request(manifest_path: &Path, package: &str) -> bool {
    let Ok(manifest) = std::fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest) else {
        return false;
    };
    let Some(name) = manifest.get("name").and_then(|value| value.as_str()) else {
        return false;
    };
    if name != npm_package_name(package) {
        return false;
    }

    match npm_package_version(package) {
        Some(version) => manifest
            .get("version")
            .and_then(|value| value.as_str())
            .is_some_and(|installed| installed == version),
        None => true,
    }
}

fn npm_global_package_dir(prefix_root: &Path, package: &str) -> PathBuf {
    package_dir_with_root(&prefix_root.join("lib").join("node_modules"), package)
}

fn package_dir_with_root(root: &Path, package: &str) -> PathBuf {
    let mut package_dir = root.to_path_buf();
    for segment in npm_package_name(package).split('/') {
        package_dir.push(segment);
    }
    package_dir
}

#[cfg(test)]
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

#[cfg(test)]
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

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileFingerprint {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        build_npm_global_recipe, capture_installation_state, file_fingerprint,
        installation_result_is_acceptable, package_bin_relative_path,
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
    fn capture_installation_state_tracks_binary_and_manifest() {
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
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"http-server","bin":{"http-server":"bin/http-server"}}"#,
        )
        .expect("write manifest");
        std::fs::write(&binary_path, "demo").expect("write binary");

        let captured = capture_installation_state(&binary_path, Some(&package_dir));
        assert!(captured.contains_key(&binary_path));
        assert!(captured.contains_key(&package_dir.join("package.json")));
        assert!(
            captured
                .get(&binary_path)
                .is_some_and(|fingerprint| fingerprint == &file_fingerprint(&binary_path))
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
        }

        let preinstall_state = capture_installation_state(&binary_path, Some(&package_dir));
        assert!(installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
            Some(&package_dir),
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

        let preinstall_state = capture_installation_state(&binary_path, None);
        assert!(!installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
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
        }

        let preinstall_state = capture_installation_state(&binary_path, Some(&package_dir));
        assert!(!installation_result_is_acceptable(
            &preinstall_state,
            &binary_path,
            "http-server@14.1.1",
            Some(&package_dir),
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
