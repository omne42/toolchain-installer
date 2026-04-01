use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use github_kit::GitHubReleaseAsset;
use omne_artifact_install_primitives::{
    install_archive_tree_from_bytes, install_binary_from_archive,
};
use omne_host_info_primitives::{
    detect_host_platform, executable_suffix_for_target, resolve_target_triple,
};
use omne_integrity_primitives::{hash_sha256, parse_sha256_digest, parse_sha256_user_input};
use omne_system_package_primitives::{
    SystemPackageManager, SystemPackageName, default_system_package_install_recipes_for_os,
    try_default_system_package_install_recipes_for_os,
};

use crate::builtin_tools::{
    ManagedBootstrapState, assess_managed_bootstrap_state, builtin_tool_destination,
    gh_release_asset_suffix_for_target, host_command_is_healthy, install_gh_from_public_release,
    install_git_from_public_release, normalize_requested_tools, replace_mingit_installation,
    select_mingit_release_asset_for_target,
};
use crate::contracts::{
    BootstrapArchiveFormat, BootstrapCommand, BootstrapSourceKind, BootstrapStatus,
    ExecutionRequest, InstallPlan, InstallPlanItem, PLAN_SCHEMA_VERSION,
};
use crate::download_sources::make_download_candidates;
use crate::error::ExitCode;
use crate::external_gateway::{
    gateway_candidate_for_git_release_download_url, make_gateway_asset_candidate,
};
use crate::install_plan::cargo_install_item_execution::execute_cargo_install_item;
use crate::install_plan::go_install_item_execution::execute_go_install_item;
use crate::install_plan::install_plan_validation::{
    validate_plan, validate_plan_with_base_dir, validate_plan_with_managed_dir,
};
use crate::installer_runtime_config::{
    DEFAULT_GITHUB_API_BASE, DEFAULT_PYPI_INDEX, DownloadPolicy, DownloadSourcePolicy,
    GatewayRoutingPolicy, GitHubReleasePolicy, InstallerRuntimeConfig, PackageIndexPolicy,
    PythonMirrorPolicy,
};
use crate::managed_toolchain::managed_environment_layout::managed_python_installation_dir;
use crate::managed_toolchain::managed_root_dir::{
    default_managed_dir_under_data_root, resolve_managed_toolchain_dir,
};
use crate::managed_toolchain::{
    execute_managed_toolchain_item, execute_uv_python_item, execute_uv_tool_item,
    find_managed_python_executable, install_uv_from_public_release, managed_uv_is_healthy,
};
use crate::plan_items::{
    CargoInstallPlanItem, CargoInstallSource, GoInstallPlanItem, GoInstallSource,
    ManagedUvPlanItem, ResolvedPlanItem, UvPythonPlanItem, UvToolPlanItem,
};

fn test_runtime_config() -> InstallerRuntimeConfig {
    InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![DEFAULT_GITHUB_API_BASE.to_string()],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        package_indexes: PackageIndexPolicy {
            indexes: vec![DEFAULT_PYPI_INDEX.to_string()],
        },
        python_mirrors: PythonMirrorPolicy {
            install_mirrors: Vec::new(),
        },
        gateway: GatewayRoutingPolicy {
            base: None,
            country: None,
        },
        download: DownloadPolicy {
            http_timeout: Duration::from_secs(5),
            max_download_bytes: None,
        },
    }
}

#[test]
fn parse_sha256_digest_accepts_valid_value() {
    let digest = parse_sha256_digest(Some(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ));
    assert_eq!(
        digest.as_ref().map(ToString::to_string).as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
}

#[test]
fn parse_sha256_user_input_accepts_raw_hex() {
    let digest =
        parse_sha256_user_input("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(
        digest.as_ref().map(ToString::to_string).as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
}

#[test]
fn make_download_candidates_prefers_gateway() {
    let out = make_download_candidates(
        "https://github.com/org/repo/releases/download/v1/x.tar.gz",
        &["https://proxy.example/".to_string()],
        Some("https://gateway.example/toolchain/gh/v1/x.tar.gz"),
    );
    assert_eq!(out[0], "https://gateway.example/toolchain/gh/v1/x.tar.gz");
    assert_eq!(
        out[1],
        "https://github.com/org/repo/releases/download/v1/x.tar.gz"
    );
}

#[test]
fn gateway_only_enabled_for_cn() {
    let cfg_cn = InstallerRuntimeConfig {
        gateway: GatewayRoutingPolicy {
            base: Some("https://gw.example".to_string()),
            country: Some("CN".to_string()),
        },
        ..test_runtime_config()
    };
    assert!(cfg_cn.gateway.use_for_git_release());

    let cfg_us = InstallerRuntimeConfig {
        gateway: GatewayRoutingPolicy {
            base: Some("https://gw.example".to_string()),
            country: Some("US".to_string()),
        },
        ..test_runtime_config()
    };
    assert!(!cfg_us.gateway.use_for_git_release());
}

#[test]
fn builtin_tool_destination_uses_git_cmd_for_windows_git() {
    let managed_dir = Path::new("/tmp/managed");
    assert_eq!(
        builtin_tool_destination("git", "x86_64-pc-windows-msvc", ".exe", managed_dir),
        managed_dir.join("git.cmd")
    );
}

#[test]
fn builtin_tool_destination_uses_binary_extension_for_other_tools() {
    let managed_dir = Path::new("/tmp/managed");
    assert_eq!(
        builtin_tool_destination("gh", "x86_64-unknown-linux-gnu", "", managed_dir),
        managed_dir.join("gh")
    );
    assert_eq!(
        builtin_tool_destination("uv", "x86_64-pc-windows-msvc", ".exe", managed_dir),
        managed_dir.join("uv.exe")
    );
}

#[test]
fn runtime_config_uses_default_package_index_only_when_none_is_configured() {
    if std::env::var_os("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES").is_some() {
        return;
    }
    let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest::default());
    assert_eq!(
        cfg.package_indexes.indexes,
        vec![DEFAULT_PYPI_INDEX.to_string()]
    );
}

#[test]
fn runtime_config_does_not_prepend_official_package_index_when_explicit_indexes_exist() {
    if std::env::var_os("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES").is_some() {
        return;
    }
    let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
        package_indexes: vec!["https://mirror.example/simple".to_string()],
        ..ExecutionRequest::default()
    });
    assert_eq!(
        cfg.package_indexes.indexes,
        vec!["https://mirror.example/simple".to_string()]
    );
}

#[test]
fn runtime_config_preserves_explicit_source_order_while_deduping() {
    if std::env::var_os("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES").is_some()
        || std::env::var_os("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES").is_some()
        || std::env::var_os("TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS").is_some()
    {
        return;
    }
    let cfg = InstallerRuntimeConfig::from_execution_request(&ExecutionRequest {
        mirror_prefixes: vec![
            "https://mirror-b.example/releases".to_string(),
            "https://mirror-a.example/releases".to_string(),
            "https://mirror-b.example/releases".to_string(),
        ],
        package_indexes: vec![
            "https://index-b.example/simple".to_string(),
            "https://index-a.example/simple".to_string(),
            "https://index-b.example/simple".to_string(),
        ],
        python_install_mirrors: vec![
            "https://python-b.example".to_string(),
            "https://python-a.example".to_string(),
            "https://python-b.example".to_string(),
        ],
        ..ExecutionRequest::default()
    });

    assert_eq!(
        cfg.download_sources.mirror_prefixes,
        vec![
            "https://mirror-b.example/releases".to_string(),
            "https://mirror-a.example/releases".to_string(),
        ]
    );
    assert_eq!(
        cfg.package_indexes.indexes,
        vec![
            "https://index-b.example/simple".to_string(),
            "https://index-a.example/simple".to_string(),
        ]
    );
    assert_eq!(
        cfg.python_mirrors.install_mirrors,
        vec![
            "https://python-b.example".to_string(),
            "https://python-a.example".to_string(),
        ]
    );
}

#[test]
fn install_plan_contract_rejects_unknown_fields_during_deserialization() {
    let err = serde_json::from_str::<InstallPlan>(
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo", "method": "uv", "unexpected": true }
  ]
}"#,
    )
    .expect_err("unknown fields should fail during deserialization");

    assert!(err.to_string().contains("unexpected"));
}

#[test]
fn assess_managed_bootstrap_state_reports_missing_install() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("uv");
    let state = assess_managed_bootstrap_state(
        "uv",
        "x86_64-unknown-linux-gnu",
        &destination,
        &managed_dir,
    );
    assert_eq!(state, ManagedBootstrapState::NeedsInstall);
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn assess_managed_bootstrap_state_ignores_unknown_tool_even_when_binary_is_healthy() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("custom-tool");
    write_executable(
        &destination,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "custom-tool 1.0.0"
  exit 0
fi
exit 0
"#,
    )
    .expect("write custom-tool");

    let state = assess_managed_bootstrap_state(
        "custom-tool",
        "x86_64-unknown-linux-gnu",
        &destination,
        &managed_dir,
    );
    assert_eq!(state, ManagedBootstrapState::NeedsInstall);
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn assess_managed_bootstrap_state_reports_healthy_binary_after_version_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("uv");
    write_executable(
        &destination,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.1.0"
  exit 0
fi
exit 2
"#,
    )
    .expect("write executable");

    let state = assess_managed_bootstrap_state(
        "uv",
        "x86_64-unknown-linux-gnu",
        &destination,
        &managed_dir,
    );
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedHealthy {
            detail: "managed binary passed --version health check".to_string()
        }
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn managed_uv_is_healthy_requires_successful_version_probe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_uv = tmp.path().join("uv");
    write_executable(
        &managed_uv,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  exit 2
fi
exit 0
"#,
    )
    .expect("write executable");

    assert!(!managed_uv_is_healthy(&managed_uv));
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn managed_uv_is_healthy_times_out_hung_version_probe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_uv = tmp.path().join("uv");
    write_executable(
        &managed_uv,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  sleep 30
  exit 0
fi
exit 0
"#,
    )
    .expect("write executable");

    assert!(!managed_uv_is_healthy(&managed_uv));
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn cargo_install_promotes_matching_staged_binary_to_requested_destination_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cargo = tmp.path().join("cargo");
    write_executable(
        &cargo,
        r#"#!/bin/sh
root=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--root" ]; then
    root="$2"
    shift 2
    continue
  fi
  if [ "$1" = "--bin" ]; then
    if [ "$2" != "alias-tool" ]; then
      echo "unexpected cargo --bin value: $2" >&2
      exit 7
    fi
    shift 2
    continue
  fi
  shift
done
mkdir -p "$root/bin"
printf 'cargo-installed' > "$root/bin/alias-tool"
chmod +x "$root/bin/alias-tool"
"#,
    )
    .expect("write cargo");
    let managed_dir = tmp.path().join("managed");
    let item = CargoInstallPlanItem {
        id: "cargo-demo".to_string(),
        source: CargoInstallSource::RegistryPackage {
            package: "demo-tool".to_string(),
            version: None,
        },
        binary_name: "alias-tool".to_string(),
        binary_name_explicit: true,
    };

    let result = with_path_prepend(tmp.path(), || {
        execute_cargo_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect("cargo install should succeed");

    let destination = managed_dir.join("bin").join("alias-tool");
    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read installed cargo binary"),
        "cargo-installed"
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn cargo_install_rejects_single_staged_binary_that_does_not_match_explicit_binary_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cargo = tmp.path().join("cargo");
    write_executable(
        &cargo,
        r#"#!/bin/sh
root=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--root" ]; then
    root="$2"
    shift 2
    continue
  fi
  if [ "$1" = "--bin" ]; then
    if [ "$2" != "alias-tool" ]; then
      echo "unexpected cargo --bin value: $2" >&2
      exit 7
    fi
    shift 2
    continue
  fi
  shift
done
mkdir -p "$root/bin"
printf 'cargo-installed' > "$root/bin/actual-tool"
chmod +x "$root/bin/actual-tool"
"#,
    )
    .expect("write cargo");
    let managed_dir = tmp.path().join("managed");
    let item = CargoInstallPlanItem {
        id: "cargo-demo".to_string(),
        source: CargoInstallSource::RegistryPackage {
            package: "demo-tool".to_string(),
            version: None,
        },
        binary_name: "alias-tool".to_string(),
        binary_name_explicit: true,
    };

    let err = with_path_prepend(tmp.path(), || {
        execute_cargo_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect_err("explicit binary name mismatch should fail");

    assert!(
        err.to_string()
            .contains("did not match the requested binary name `alias-tool`")
    );
    assert!(
        !managed_dir.join("bin").join("alias-tool").exists(),
        "mismatched staged binary must not be promoted into the managed destination"
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn cargo_install_restores_previous_binary_when_staged_output_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cargo = tmp.path().join("cargo");
    write_executable(
        &cargo,
        r#"#!/bin/sh
root=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--root" ]; then
    root="$2"
    shift 2
    continue
  fi
  shift
done
mkdir -p "$root/bin"
exit 0
"#,
    )
    .expect("write cargo");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("bin").join("alias-tool");
    let stale_binary = "#!/bin/sh\necho stale-cargo\n";
    write_executable(&destination, stale_binary).expect("write stale cargo binary");
    let item = CargoInstallPlanItem {
        id: "cargo-demo".to_string(),
        source: CargoInstallSource::RegistryPackage {
            package: "demo-tool".to_string(),
            version: None,
        },
        binary_name: "alias-tool".to_string(),
        binary_name_explicit: true,
    };

    let err = with_path_prepend(tmp.path(), || {
        execute_cargo_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect_err("missing staged cargo binary should fail");

    assert!(err.to_string().contains("produced no staged binary"));
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read restored cargo binary"),
        stale_binary
    );
    assert!(
        !destination
            .with_file_name("alias-tool.toolchain-installer-backup")
            .exists()
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn cargo_install_replaces_previous_directory_destination_and_cleans_backup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cargo = tmp.path().join("cargo");
    write_executable(
        &cargo,
        r#"#!/bin/sh
root=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--root" ]; then
    root="$2"
    shift 2
    continue
  fi
  shift
done
mkdir -p "$root/bin"
printf 'cargo-installed' > "$root/bin/alias-tool"
chmod +x "$root/bin/alias-tool"
"#,
    )
    .expect("write cargo");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("bin").join("alias-tool");
    std::fs::create_dir_all(&destination).expect("create old directory destination");
    std::fs::write(destination.join("old"), "stale").expect("write stale directory content");
    let item = CargoInstallPlanItem {
        id: "cargo-demo".to_string(),
        source: CargoInstallSource::RegistryPackage {
            package: "demo-tool".to_string(),
            version: None,
        },
        binary_name: "alias-tool".to_string(),
        binary_name_explicit: true,
    };

    let result = with_path_prepend(tmp.path(), || {
        execute_cargo_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect("cargo install should replace directory destination");

    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read installed cargo binary"),
        "cargo-installed"
    );
    assert!(
        !destination
            .with_file_name("alias-tool.toolchain-installer-backup")
            .exists()
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn cargo_install_recovers_from_stale_backup_before_reinstall() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cargo = tmp.path().join("cargo");
    write_executable(
        &cargo,
        r#"#!/bin/sh
root=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--root" ]; then
    root="$2"
    shift 2
    continue
  fi
  shift
done
mkdir -p "$root/bin"
printf 'cargo-installed' > "$root/bin/alias-tool"
chmod +x "$root/bin/alias-tool"
"#,
    )
    .expect("write cargo");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("bin").join("alias-tool");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create destination parent");
    write_executable(&destination, "#!/bin/sh\necho current-cargo\n").expect("write current");
    let stale_backup = destination.with_file_name("alias-tool.toolchain-installer-backup");
    std::fs::write(&stale_backup, "stale-backup").expect("write stale backup");
    let item = CargoInstallPlanItem {
        id: "cargo-demo".to_string(),
        source: CargoInstallSource::RegistryPackage {
            package: "demo-tool".to_string(),
            version: None,
        },
        binary_name: "alias-tool".to_string(),
        binary_name_explicit: true,
    };

    let result = with_path_prepend(tmp.path(), || {
        execute_cargo_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect("cargo install should recover from stale backup");

    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read installed cargo binary"),
        "cargo-installed"
    );
    assert!(
        !stale_backup.exists(),
        "canonical backup should be consumed or removed during recovery"
    );
    let quarantined = std::fs::read_dir(destination.parent().expect("destination parent"))
        .expect("read managed bin dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.contains(".toolchain-installer-backup.stale-"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        quarantined.len(),
        1,
        "stale canonical backup should be quarantined once"
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn go_install_promotes_single_staged_binary_to_requested_destination_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let go = tmp.path().join("go");
    write_executable(
        &go,
        r#"#!/bin/sh
mkdir -p "$GOBIN"
printf 'go-installed' > "$GOBIN/actual-tool"
chmod +x "$GOBIN/actual-tool"
"#,
    )
    .expect("write go");
    let managed_dir = tmp.path().join("managed");
    let item = GoInstallPlanItem {
        id: "go-demo".to_string(),
        source: GoInstallSource::PackageSpec("example.com/demo/cmd/demo@latest".to_string()),
        binary_name: "alias-tool".to_string(),
    };

    let result = with_path_prepend(tmp.path(), || {
        execute_go_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect("go install should succeed");

    let destination = managed_dir.join("alias-tool");
    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read installed go binary"),
        "go-installed"
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn go_install_recovers_from_stale_backup_before_reinstall() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let go = tmp.path().join("go");
    write_executable(
        &go,
        r#"#!/bin/sh
mkdir -p "$GOBIN"
printf 'go-installed' > "$GOBIN/alias-tool"
chmod +x "$GOBIN/alias-tool"
"#,
    )
    .expect("write go");
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("create managed dir");
    let destination = managed_dir.join("alias-tool");
    write_executable(&destination, "#!/bin/sh\necho current-go\n").expect("write current");
    let stale_backup = destination.with_file_name("alias-tool.toolchain-installer-backup");
    std::fs::write(&stale_backup, "stale-backup").expect("write stale backup");
    let item = GoInstallPlanItem {
        id: "go-demo".to_string(),
        source: GoInstallSource::PackageSpec("example.com/demo/cmd/demo@latest".to_string()),
        binary_name: "alias-tool".to_string(),
    };

    let result = with_path_prepend(tmp.path(), || {
        execute_go_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect("go install should recover from stale backup");

    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read installed go binary"),
        "go-installed"
    );
    assert!(
        !stale_backup.exists(),
        "canonical backup should be consumed or removed during recovery"
    );
    let quarantined = std::fs::read_dir(&managed_dir)
        .expect("read managed dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.contains(".toolchain-installer-backup.stale-"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        quarantined.len(),
        1,
        "stale canonical backup should be quarantined once"
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn go_install_restores_previous_binary_when_staged_output_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let go = tmp.path().join("go");
    write_executable(
        &go,
        r#"#!/bin/sh
mkdir -p "$GOBIN"
exit 0
"#,
    )
    .expect("write go");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("alias-tool");
    let stale_binary = "#!/bin/sh\necho stale-go\n";
    write_executable(&destination, stale_binary).expect("write stale go binary");
    let item = GoInstallPlanItem {
        id: "go-demo".to_string(),
        source: GoInstallSource::PackageSpec("example.com/demo/cmd/demo@latest".to_string()),
        binary_name: "alias-tool".to_string(),
    };

    let err = with_path_prepend(tmp.path(), || {
        execute_go_install_item(&item, "x86_64-unknown-linux-gnu", &managed_dir)
    })
    .expect_err("missing staged go binary should fail");

    assert!(err.to_string().contains("produced no staged binary"));
    assert_eq!(
        std::fs::read_to_string(&destination).expect("read restored go binary"),
        stale_binary
    );
    assert!(
        !destination
            .with_file_name("alias-tool.toolchain-installer-backup")
            .exists()
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn host_command_is_healthy_times_out_hung_version_probe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_executable(
        &tmp.path().join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  sleep 30
  exit 0
fi
exit 0
"#,
    )
    .expect("write uv");

    with_path_prepend(tmp.path(), || {
        assert!(!host_command_is_healthy("uv"));
    });
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn find_managed_python_executable_accepts_major_only_version_selector() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let python = managed_dir.join("python3");
    write_executable(
        &python,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "Python 3.13.2"
  exit 0
fi
exit 2
"#,
    )
    .expect("write python");

    let found = find_managed_python_executable(&managed_dir, "3", "x86_64-unknown-linux-gnu");
    assert_eq!(found.as_deref(), Some(python.as_path()));
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn find_managed_python_executable_does_not_use_substring_version_matching() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let python = managed_dir.join("python3.10");
    write_executable(
        &python,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "Python 3.10.8"
  exit 0
fi
exit 2
"#,
    )
    .expect("write python");

    let found = find_managed_python_executable(&managed_dir, "3.1", "x86_64-unknown-linux-gnu");
    assert!(found.is_none());
}

#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_git_launcher_without_payload() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    std::fs::create_dir_all(&managed_dir).expect("create managed dir");
    std::fs::write(&destination, "@echo off\r\n").expect("write git launcher");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git launcher at {} does not point to a MinGit payload",
                destination.display()
            )
        }
    );
}

#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_git_when_runtime_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let payload = managed_dir
        .join("git-portable")
        .join("PortableGit")
        .join("cmd");
    std::fs::create_dir_all(&payload).expect("create payload dir");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\cmd\\git.exe\" %*\r\n",
    )
    .expect("write launcher");
    std::fs::write(payload.join("git.exe"), b"MZ").expect("write git.exe");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git payload is missing required runtime {}",
                managed_dir
                    .join("git-portable")
                    .join("PortableGit")
                    .join("mingw64")
                    .join("bin")
                    .join("msys-2.0.dll")
                    .display()
            )
        }
    );
}

#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_cmd_git_when_runtime_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let payload = managed_dir
        .join("git-portable")
        .join("PortableGit")
        .join("cmd");
    std::fs::create_dir_all(&payload).expect("create payload dir");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\cmd\\git.exe\" %*\r\n",
    )
    .expect("write launcher");
    std::fs::write(payload.join("git.exe"), b"MZ").expect("write git.exe");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git payload is missing required runtime {}",
                managed_dir
                    .join("git-portable")
                    .join("PortableGit")
                    .join("mingw64")
                    .join("bin")
                    .join("msys-2.0.dll")
                    .display()
            )
        }
    );
}

#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_git_when_launcher_escapes_managed_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&managed_dir).expect("create managed dir");
    std::fs::create_dir_all(&outside).expect("create outside dir");
    std::fs::write(outside.join("git.exe"), b"MZ").expect("write outside git.exe");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0..\\outside\\git.exe\" %*\r\n",
    )
    .expect("write launcher");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    match state {
        ManagedBootstrapState::ManagedBroken { detail } => {
            assert_eq!(
                detail.replace('\\', "/"),
                "managed git launcher points outside managed root with payload target `../outside/git.exe`"
            );
        }
        other => panic!("expected ManagedBroken state, got {other:?}"),
    }
}

#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_git_when_launcher_escapes_git_portable_root()
 {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let other = managed_dir.join("other");
    std::fs::create_dir_all(&other).expect("create other dir");
    std::fs::write(other.join("git.exe"), b"MZ").expect("write other git.exe");
    std::fs::write(&destination, "@echo off\r\n\"%~dp0other\\git.exe\" %*\r\n")
        .expect("write launcher");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    match state {
        ManagedBootstrapState::ManagedBroken { detail } => {
            assert_eq!(
                detail.replace('\\', "/"),
                "managed git launcher points outside managed git-portable root with payload target `other/git.exe`"
            );
        }
        other => panic!("expected ManagedBroken state, got {other:?}"),
    }
}

#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_git_when_launcher_escapes_portablegit_payload_root()
 {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let other = managed_dir.join("git-portable").join("other");
    std::fs::create_dir_all(&other).expect("create other dir");
    std::fs::write(other.join("git.exe"), b"MZ").expect("write other git.exe");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\other\\git.exe\" %*\r\n",
    )
    .expect("write launcher");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    match state {
        ManagedBootstrapState::ManagedBroken { detail } => {
            assert_eq!(
                detail.replace('\\', "/"),
                "managed git launcher points outside managed PortableGit payload root with payload target `git-portable/other/git.exe`"
            );
        }
        other => panic!("expected ManagedBroken state, got {other:?}"),
    }
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn host_command_is_healthy_rejects_broken_supported_host_binary() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_executable(
        &tmp.path().join("uv"),
        r#"#!/bin/sh
exit 42
"#,
    )
    .expect("write uv");

    with_path_prepend(tmp.path(), || {
        assert!(!host_command_is_healthy("uv"));
    });
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn host_command_is_healthy_rejects_mismatched_version_prefix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_executable(
        &tmp.path().join("gh"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "definitely-not-gh 1.0.0"
  exit 0
fi
exit 0
"#,
    )
    .expect("write gh");

    with_path_prepend(tmp.path(), || {
        assert!(!host_command_is_healthy("gh"));
    });
}

#[cfg(unix)]
#[test]
fn host_command_is_healthy_rejects_non_executable_supported_host_file() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("tempdir");
    let plain = tmp.path().join("uv");
    std::fs::write(&plain, "not executable").expect("write plain file");
    let mut permissions = std::fs::metadata(&plain)
        .expect("stat plain file")
        .permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(&plain, permissions).expect("chmod plain file");

    with_path_override(tmp.path(), || {
        assert!(!host_command_is_healthy("uv"));
    });
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn host_command_is_healthy_rejects_unknown_tool_even_when_host_binary_is_healthy() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_executable(
        &tmp.path().join("custom-tool"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "custom-tool 1.0.0"
  exit 0
fi
exit 0
"#,
    )
    .expect("write custom-tool");

    with_path_prepend(tmp.path(), || {
        assert!(!host_command_is_healthy("custom-tool"));
    });
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn bootstrap_reports_unsupported_tool_even_when_host_path_contains_same_name() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    write_executable(
        &tmp.path().join("custom-tool"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "custom-tool 1.0.0"
  exit 0
fi
exit 0
"#,
    )?;

    let result = with_path_prepend(tmp.path(), || {
        tokio::runtime::Runtime::new()
            .expect("create runtime")
            .block_on(crate::bootstrap(&BootstrapCommand {
                execution: ExecutionRequest {
                    managed_dir: Some(tmp.path().join("managed")),
                    ..ExecutionRequest::default()
                },
                tools: vec!["custom-tool".to_string()],
            }))
    })?;

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].tool, "custom-tool");
    assert_eq!(result.items[0].status, BootstrapStatus::Unsupported);
    assert_ne!(result.items[0].status, BootstrapStatus::Present);
    Ok(())
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn bootstrap_reports_unsupported_tool_even_when_managed_dir_contains_healthy_binary()
-> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("custom-tool"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "custom-tool 1.0.0"
  exit 0
fi
exit 0
"#,
    )?;

    let result = tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(crate::bootstrap(&BootstrapCommand {
            execution: ExecutionRequest {
                managed_dir: Some(managed_dir),
                ..ExecutionRequest::default()
            },
            tools: vec!["custom-tool".to_string()],
        }))?;

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].tool, "custom-tool");
    assert_eq!(result.items[0].status, BootstrapStatus::Unsupported);
    assert_ne!(result.items[0].status, BootstrapStatus::Installed);
    Ok(())
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn assess_managed_bootstrap_state_reports_broken_windows_git_when_payload_fails_version_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let payload = managed_dir
        .join("git-portable")
        .join("PortableGit")
        .join("mingw64")
        .join("bin");
    std::fs::create_dir_all(&payload).expect("create payload dir");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\mingw64\\bin\\git.exe\" %*\r\n",
    )
    .expect("write launcher");
    write_executable(
        &payload.join("git.exe"),
        r#"#!/bin/sh
exit 2
"#,
    )
    .expect("write git.exe");
    std::fs::write(payload.join("msys-2.0.dll"), b"dll").expect("write runtime");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedBroken {
            detail: format!(
                "managed git payload {} failed --version health check",
                payload.join("git.exe").display()
            )
        }
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn assess_managed_bootstrap_state_reports_healthy_windows_git_launcher() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let payload = managed_dir
        .join("git-portable")
        .join("PortableGit")
        .join("mingw64")
        .join("bin");
    std::fs::create_dir_all(&payload).expect("create payload dir");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\mingw64\\bin\\git.exe\" %*\r\n",
    )
    .expect("write launcher");
    write_executable(
        &payload.join("git.exe"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "git version 2.53.0.windows.1"
  exit 0
fi
exit 2
"#,
    )
    .expect("write git.exe");
    std::fs::write(payload.join("msys-2.0.dll"), b"dll").expect("write runtime");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedHealthy {
            detail: format!(
                "managed git launcher points to healthy MinGit payload {} under {}",
                managed_dir
                    .join("git-portable")
                    .join("PortableGit")
                    .join("mingw64")
                    .join("bin")
                    .join("git.exe")
                    .display(),
                managed_dir.join("git-portable").display()
            )
        }
    );
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[test]
fn assess_managed_bootstrap_state_reports_healthy_windows_cmd_git_launcher() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let managed_dir = tmp.path().join("managed");
    let destination = managed_dir.join("git.cmd");
    let payload = managed_dir
        .join("git-portable")
        .join("PortableGit")
        .join("cmd");
    std::fs::create_dir_all(&payload).expect("create payload dir");
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\cmd\\git.exe\" %*\r\n",
    )
    .expect("write launcher");
    write_executable(
        &payload.join("git.exe"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "git version 2.53.0.windows.1"
  exit 0
fi
exit 2
"#,
    )
    .expect("write cmd git.exe");
    let runtime_dir = managed_dir
        .join("git-portable")
        .join("PortableGit")
        .join("mingw64")
        .join("bin");
    std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
    std::fs::write(runtime_dir.join("msys-2.0.dll"), b"dll").expect("write runtime");

    let state =
        assess_managed_bootstrap_state("git", "x86_64-pc-windows-msvc", &destination, &managed_dir);
    assert_eq!(
        state,
        ManagedBootstrapState::ManagedHealthy {
            detail: format!(
                "managed git launcher points to healthy MinGit payload {} under {}",
                payload.join("git.exe").display(),
                managed_dir.join("git-portable").display()
            )
        }
    );
}

#[test]
fn installer_errors_preserve_freeform_user_text() {
    let err = validate_plan(
        &InstallPlan {
            schema_version: Some(PLAN_SCHEMA_VERSION),
            items: vec![InstallPlanItem {
                id: "demo".to_string(),
                method: "unknown".to_string(),
                version: None,
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: None,
                package: None,
                manager: None,
                python: None,
            }],
        },
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("unknown method should be rejected");

    assert_eq!(err.exit_code(), ExitCode::Usage);
    assert!(err.to_string().contains("unsupported method `unknown`"));
}

#[test]
fn gateway_candidate_for_git_release_download_url_parses_exact_git_release_url() {
    let cfg = InstallerRuntimeConfig {
        gateway: GatewayRoutingPolicy {
            base: Some("https://gw.example".to_string()),
            country: Some("CN".to_string()),
        },
        ..test_runtime_config()
    };
    let candidate = gateway_candidate_for_git_release_download_url(
        &cfg,
        "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip",
    )
    .expect("candidate");
    assert_eq!(
        candidate,
        "https://gw.example/toolchain/git/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip"
    );
}

#[test]
fn gateway_candidate_for_git_release_download_url_ignores_query_and_fragment() {
    let cfg = InstallerRuntimeConfig {
        gateway: GatewayRoutingPolicy {
            base: Some("https://gw.example".to_string()),
            country: Some("CN".to_string()),
        },
        ..test_runtime_config()
    };
    let candidate = gateway_candidate_for_git_release_download_url(
        &cfg,
        "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip?download=1#fragment",
    )
    .expect("candidate");
    assert_eq!(
        candidate,
        "https://gw.example/toolchain/git/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip"
    );
}

#[test]
fn select_mingit_release_asset_prefers_busybox_on_x64() {
    let assets = vec![
        GitHubReleaseAsset {
            name: "MinGit-2.53.0-64-bit.zip".to_string(),
            browser_download_url: "https://example.invalid/a.zip".to_string(),
            digest: Some(
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            ),
        },
        GitHubReleaseAsset {
            name: "MinGit-2.53.0-busybox-64-bit.zip".to_string(),
            browser_download_url: "https://example.invalid/b.zip".to_string(),
            digest: Some(
                "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
            ),
        },
    ];
    let selected = select_mingit_release_asset_for_target(&assets, "x86_64-pc-windows-msvc")
        .expect("selected asset");
    assert_eq!(selected.name, "MinGit-2.53.0-busybox-64-bit.zip");
}

#[test]
fn system_recipes_cover_linux() {
    let package = SystemPackageName::new("git").expect("valid package name");
    let recipes = default_system_package_install_recipes_for_os("linux", &package);
    assert!(!recipes.is_empty());
    assert!(recipes.iter().any(|recipe| recipe.program == "apt-get"));
}

#[test]
fn toolchain_installer_composes_current_host_system_recipes() {
    let package = SystemPackageName::new("git").expect("valid package name");
    let _ = detect_host_platform().map(|platform| {
        default_system_package_install_recipes_for_os(
            platform.operating_system().as_str(),
            &package,
        )
    });
}

#[test]
fn system_package_manager_rejects_unknown_input() {
    assert_eq!(SystemPackageManager::parse("unknown"), None);
}

#[test]
fn system_package_manager_accepts_only_canonical_names() {
    assert_eq!(SystemPackageManager::parse("apt"), None);
    assert_eq!(
        SystemPackageManager::parse("apt-get"),
        Some(SystemPackageManager::AptGet)
    );
}

#[test]
fn system_package_recipe_helpers_match_runtime_validation_contract() {
    let package = SystemPackageName::new("git").expect("valid package name");
    let apt_recipe = SystemPackageManager::AptGet.install_recipe(&package);
    assert_eq!(apt_recipe.program, "apt-get");
    assert_eq!(
        apt_recipe.args,
        vec![
            "install".to_string(),
            "-y".to_string(),
            "--".to_string(),
            "git".to_string()
        ]
    );

    assert_eq!(
        SystemPackageName::new("git core")
            .expect_err("whitespace should be rejected")
            .to_string(),
        "package name must not contain whitespace"
    );
    assert_eq!(
        SystemPackageName::new("../git")
            .expect_err("path separators should be rejected")
            .to_string(),
        "package name must not contain path separators"
    );
    assert_eq!(
        SystemPackageName::new("-git")
            .expect_err("option-like names should be rejected")
            .to_string(),
        "package name must not look like a command-line option"
    );
}

#[test]
fn system_package_try_helpers_still_reject_invalid_package_names() {
    assert!(
        SystemPackageManager::AptGet
            .try_install_recipe("git core")
            .is_err()
    );
    assert!(try_default_system_package_install_recipes_for_os("linux", "../git").is_err());
}

#[tokio::test]
async fn install_gh_from_public_release_mock_api() -> anyhow::Result<()> {
    let archive_name = "gh_9.9.9_linux_amd64.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "gh_9.9.9_linux_amd64/bin/gh",
        b"#!/bin/sh\necho mock-gh\n".as_slice(),
        0o755,
    )])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "v9.9.9",
        "body": "x".repeat(20 * 1024),
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/cli/cli/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("gh");

    let source =
        install_gh_from_public_release("x86_64-unknown-linux-gnu", "", &destination, &cfg, &client)
            .await?;
    assert_eq!(source.locator, format!("{base}/asset/{archive_name}"));
    assert_eq!(source.source_kind, BootstrapSourceKind::Canonical);
    assert_eq!(
        source
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((BootstrapArchiveFormat::TarGz, "gh_9.9.9_linux_amd64/bin/gh"))
    );
    let installed = std::fs::read_to_string(&destination)?;
    assert!(installed.contains("mock-gh"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn install_gh_from_public_release_windows_zip_uses_bin_hint() -> anyhow::Result<()> {
    let archive_name = "gh_9.9.9_windows_amd64.zip";
    let archive_bytes = make_zip_archive(&[("bin/gh.exe", b"MZ".as_slice(), 0o755)])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "v9.9.9",
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/cli/cli/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("gh.exe");

    let source = install_gh_from_public_release(
        "x86_64-pc-windows-msvc",
        ".exe",
        &destination,
        &cfg,
        &client,
    )
    .await?;
    assert_eq!(source.locator, format!("{base}/asset/{archive_name}"));
    assert_eq!(source.source_kind, BootstrapSourceKind::Canonical);
    assert_eq!(
        source
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((BootstrapArchiveFormat::Zip, "bin/gh.exe"))
    );
    let installed = std::fs::read(&destination)?;
    assert_eq!(installed, b"MZ");

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn install_git_from_public_release_windows_zip_builds_cmd_launcher() -> anyhow::Result<()> {
    let archive_name = "MinGit-2.53.0-busybox-64-bit.zip";
    let archive_bytes = make_zip_archive(&[
        ("PortableGit/cmd/git.exe", b"MZ".as_slice(), 0o755),
        ("PortableGit/mingw64/bin/git.exe", b"MZ2".as_slice(), 0o755),
        (
            "PortableGit/mingw64/bin/msys-2.0.dll",
            b"dll".as_slice(),
            0o644,
        ),
    ])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "v2.53.0.windows.1",
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/git-for-windows/git/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("git.cmd");

    let source =
        install_git_from_public_release("x86_64-pc-windows-msvc", &destination, &cfg, &client)
            .await?;
    assert_eq!(source.locator, format!("{base}/asset/{archive_name}"));
    assert_eq!(source.source_kind, BootstrapSourceKind::Canonical);
    assert_eq!(
        source
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((BootstrapArchiveFormat::Zip, "PortableGit/cmd/git.exe"))
    );
    let launcher = std::fs::read_to_string(&destination)?;
    assert!(launcher.contains("git-portable\\PortableGit\\cmd\\git.exe"));
    assert!(
        tmp.path()
            .join("git-portable")
            .join("PortableGit")
            .join("mingw64")
            .join("bin")
            .join("msys-2.0.dll")
            .exists()
    );

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn install_git_from_public_release_windows_zip_accepts_mingw64_fallback() -> anyhow::Result<()>
{
    let archive_name = "MinGit-2.53.0.2-busybox-64-bit.zip";
    let archive_bytes = make_zip_archive(&[
        ("PortableGit/mingw64/bin/git.exe", b"MZ".as_slice(), 0o755),
        (
            "PortableGit/mingw64/bin/msys-2.0.dll",
            b"dll".as_slice(),
            0o644,
        ),
    ])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "v2.53.0.windows.2",
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/git-for-windows/git/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("git.cmd");

    let source =
        install_git_from_public_release("x86_64-pc-windows-msvc", &destination, &cfg, &client)
            .await?;
    assert_eq!(source.locator, format!("{base}/asset/{archive_name}"));
    assert_eq!(source.source_kind, BootstrapSourceKind::Canonical);
    assert_eq!(
        source
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((
            BootstrapArchiveFormat::Zip,
            "PortableGit/mingw64/bin/git.exe"
        ))
    );
    let launcher = std::fs::read_to_string(&destination)?;
    assert!(launcher.contains("git-portable\\PortableGit\\mingw64\\bin\\git.exe"));
    assert!(
        tmp.path()
            .join("git-portable")
            .join("PortableGit")
            .join("mingw64")
            .join("bin")
            .join("msys-2.0.dll")
            .exists()
    );

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn install_git_from_public_release_preserves_existing_install_on_failed_update()
-> anyhow::Result<()> {
    let archive_name = "MinGit-2.53.0-busybox-64-bit.zip";
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "v2.53.0.windows.1",
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{}", sha256_hex(b"not a zip archive"))
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/git-for-windows/git/releases/latest".to_string(),
        release_body,
    );
    routes.insert(
        format!("/asset/{archive_name}"),
        b"not a zip archive".to_vec(),
    );
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("git.cmd");
    let existing_git = tmp
        .path()
        .join("git-portable")
        .join("PortableGit")
        .join("cmd")
        .join("git.exe");
    std::fs::create_dir_all(existing_git.parent().expect("git parent"))?;
    std::fs::write(&existing_git, b"OLD-GIT")?;
    std::fs::write(
        &destination,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\cmd\\git.exe\" %*\r\n",
    )?;

    let err =
        install_git_from_public_release("x86_64-pc-windows-msvc", &destination, &cfg, &client)
            .await
            .expect_err("invalid archive should fail");
    assert!(err.detail().contains("invalid"));
    assert_eq!(std::fs::read(&existing_git)?, b"OLD-GIT");
    assert_eq!(
        std::fs::read_to_string(&destination)?,
        "@echo off\r\n\"%~dp0git-portable\\PortableGit\\cmd\\git.exe\" %*\r\n"
    );
    assert!(!tmp.path().join("git-portable.stage").exists());
    assert!(!tmp.path().join("git-portable.backup").exists());

    handle.join().expect("mock server thread join");
    Ok(())
}

#[test]
fn replace_mingit_installation_swaps_staging_and_keeps_backup_for_followup_steps()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let portable_root = temp.path().join("git-portable");
    let staging_root = temp.path().join("git-portable.stage");
    let backup_root = temp.path().join("git-portable.backup");

    let old_git = portable_root
        .join("PortableGit")
        .join("cmd")
        .join("git.exe");
    std::fs::create_dir_all(old_git.parent().expect("old git parent"))?;
    std::fs::write(&old_git, b"OLD-GIT")?;

    let new_git = staging_root.join("PortableGit").join("cmd").join("git.exe");
    std::fs::create_dir_all(new_git.parent().expect("new git parent"))?;
    std::fs::write(&new_git, b"NEW-GIT")?;

    replace_mingit_installation(&portable_root, &staging_root, &backup_root)?;

    assert_eq!(std::fs::read(&old_git)?, b"NEW-GIT");
    assert!(!staging_root.exists());
    assert_eq!(
        std::fs::read(backup_root.join("PortableGit").join("cmd").join("git.exe"))?,
        b"OLD-GIT"
    );
    Ok(())
}

#[tokio::test]
async fn install_uv_from_mock_release_api() -> anyhow::Result<()> {
    let archive_name = "uv-x86_64-unknown-linux-gnu.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "uv-x86_64-unknown-linux-gnu/uv",
        b"#!/bin/sh\necho uv 0.11.0\n".as_slice(),
        0o755,
    )])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "0.11.0",
        "body": "x".repeat(20 * 1024),
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/astral-sh/uv/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("uv");

    let source =
        install_uv_from_public_release("x86_64-unknown-linux-gnu", &destination, &cfg, &client)
            .await?;
    assert_eq!(source.locator, format!("{base}/asset/{archive_name}"));
    assert_eq!(source.source_kind, BootstrapSourceKind::Canonical);
    assert_eq!(
        source
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((
            BootstrapArchiveFormat::TarGz,
            "uv-x86_64-unknown-linux-gnu/uv"
        ))
    );
    let installed = std::fs::read_to_string(&destination)?;
    assert!(installed.contains("uv 0.11.0"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn install_uv_from_mock_windows_zip_requires_archive_root_binary() -> anyhow::Result<()> {
    let archive_name = "uv-x86_64-pc-windows-msvc.zip";
    let archive_bytes = make_zip_archive(&[("uv.exe", b"mock-windows-uv".as_slice(), 0o755)])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "0.11.0",
        "body": "x".repeat(20 * 1024),
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/astral-sh/uv/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        download_sources: DownloadSourcePolicy {
            mirror_prefixes: Vec::new(),
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("uv.exe");

    let err = install_uv_from_public_release("x86_64-pc-windows-msvc", &destination, &cfg, &client)
        .await
        .expect_err("root-level Windows uv binary should be rejected");
    assert!(
        err.detail().contains("binary `uv.exe` not found"),
        "unexpected error: {err:?}"
    );

    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock executable is unix-specific")]
#[tokio::test]
async fn execute_managed_uv_item_reinstalls_broken_existing_binary() -> anyhow::Result<()> {
    let archive_name = "uv-x86_64-unknown-linux-gnu.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "uv-x86_64-unknown-linux-gnu/uv",
        b"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo uv 0.11.0\n  exit 0\nfi\nexit 2\n"
            .as_slice(),
        0o755,
    )])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "0.11.0",
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/astral-sh/uv/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  exit 42\nfi\nexit 42\n",
    )?;

    let result = execute_managed_toolchain_item(
        &ResolvedPlanItem::Uv(ManagedUvPlanItem {
            id: "uv".to_string(),
        }),
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;
    assert_eq!(result.status, BootstrapStatus::Installed);
    assert_eq!(result.source_kind, Some(BootstrapSourceKind::Canonical));
    assert!(
        result
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("reinstalled managed uv after broken binary")
    );
    assert!(std::fs::read_to_string(managed_dir.join("uv"))?.contains("uv 0.11.0"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_rejects_download_over_configured_size_limit() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/asset/demo.bin".to_string(), b"0123456789".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/demo.bin")),
            sha256: None,
            archive_binary: None,
            binary_name: Some("demo".to_string()),
            destination: Some("demo".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir),
        max_download_bytes: Some(4),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Failed);
    assert_eq!(
        result.items[0].error_code.as_deref(),
        Some("download_failed")
    );
    assert!(
        result.items[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("configured max download size 4")
    );

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_installs_non_archive_release_with_sha256() -> anyhow::Result<()> {
    let binary_bytes = b"#!/bin/sh\necho streamed-demo\n".to_vec();
    let digest = sha256_hex(&binary_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/asset/demo".to_string(), binary_bytes.clone());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/demo")),
            sha256: Some(digest),
            archive_binary: None,
            binary_name: Some("demo".to_string()),
            destination: Some("demo".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir.clone()),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Installed);
    assert_eq!(
        result.items[0].source.as_deref(),
        Some(format!("{base}/asset/demo").as_str())
    );
    assert_eq!(
        result.items[0].source_kind,
        Some(BootstrapSourceKind::Canonical)
    );
    let installed = std::fs::read_to_string(managed_dir.join("demo"))?;
    assert!(installed.contains("streamed-demo"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_installs_archive_release_and_reports_archive_match()
-> anyhow::Result<()> {
    let archive_name = "demo-release.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "demo-release/bin/demo",
        b"#!/bin/sh\necho archive-demo\n".as_slice(),
        0o755,
    )])?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/{archive_name}")),
            sha256: None,
            archive_binary: None,
            binary_name: Some("demo".to_string()),
            destination: Some("demo".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir.clone()),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Installed);
    assert_eq!(
        result.items[0].source.as_deref(),
        Some(format!("{base}/asset/{archive_name}").as_str())
    );
    assert_eq!(
        result.items[0]
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((BootstrapArchiveFormat::TarGz, "demo-release/bin/demo"))
    );
    let installed = std::fs::read_to_string(managed_dir.join("demo"))?;
    assert!(installed.contains("archive-demo"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_installs_archive_release_when_url_has_query() -> anyhow::Result<()> {
    let archive_name = "demo-query.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "demo-query/bin/demo",
        b"#!/bin/sh\necho archive-demo-query\n".as_slice(),
        0o755,
    )])?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    routes.insert(
        format!("/asset/{archive_name}?download=1"),
        routes[&format!("/asset/{archive_name}")].clone(),
    );
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/{archive_name}?download=1")),
            sha256: None,
            archive_binary: None,
            binary_name: Some("demo".to_string()),
            destination: Some("demo".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir.clone()),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(
        result.items[0].status,
        BootstrapStatus::Installed,
        "detail={:?}",
        result.items[0].detail
    );
    assert_eq!(
        result.items[0]
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((BootstrapArchiveFormat::TarGz, "demo-query/bin/demo"))
    );
    let installed = std::fs::read_to_string(managed_dir.join("demo"))?;
    assert!(installed.contains("archive-demo-query"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_redacts_release_source_url_in_result() -> anyhow::Result<()> {
    let binary_bytes = b"#!/bin/sh\necho redact-release\n".to_vec();

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://127.0.0.1:{}/asset/demo", addr.port());
    let request_url = format!("{base}?token=secret#ignored");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/asset/demo?token=secret".to_string(), binary_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(request_url),
            sha256: None,
            archive_binary: None,
            binary_name: Some("demo".to_string()),
            destination: Some("demo".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Installed);
    assert_eq!(result.items[0].source.as_deref(), Some(base.as_str()));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_installs_archive_release_with_relative_archive_binary_hint()
-> anyhow::Result<()> {
    let archive_name = "node-v22.14.0-linux-x64.tar.xz";
    let archive_bytes = make_tar_xz_archive(&[(
        "node-v22.14.0-linux-x64/bin/node",
        b"#!/bin/sh\necho node-relative-hint\n".as_slice(),
        0o755,
    )])?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "node-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/{archive_name}")),
            sha256: None,
            archive_binary: Some("node-v22.14.0-linux-x64/bin/node".to_string()),
            binary_name: Some("node".to_string()),
            destination: Some("node".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir.clone()),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Installed);
    assert_eq!(
        result.items[0]
            .archive_match
            .as_ref()
            .map(|matched| (matched.format, matched.path.as_str())),
        Some((
            BootstrapArchiveFormat::TarXz,
            "node-v22.14.0-linux-x64/bin/node"
        ))
    );
    let installed = std::fs::read_to_string(managed_dir.join("node"))?;
    assert!(installed.contains("node-relative-hint"));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_redacts_archive_tree_release_source_url_in_result() -> anyhow::Result<()>
{
    let archive_name = "demo-tree-redact.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "demo-tree-redact/bin/demo",
        b"#!/bin/sh\necho archive-tree-redact\n".as_slice(),
        0o755,
    )])?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://127.0.0.1:{}/asset/{archive_name}", addr.port());
    let request_url = format!(
        "http://user:secret@127.0.0.1:{}/asset/{archive_name}?download=1#ignored",
        addr.port()
    );
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/asset/{archive_name}?download=1"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-tree".to_string(),
            method: "archive_tree_release".to_string(),
            version: None,
            url: Some(request_url),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("tree".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Installed);
    assert_eq!(result.items[0].source.as_deref(), Some(base.as_str()));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_rejects_archive_release_with_unrooted_archive_binary_hint()
-> anyhow::Result<()> {
    let archive_name = "7z2600-linux-x64.tar.xz";
    let archive_bytes = make_tar_xz_archive(&[(
        "7z2600-linux-x64/7zz",
        b"#!/bin/sh\necho root-7zz\n".as_slice(),
        0o755,
    )])?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/asset/{archive_name}"), archive_bytes.clone());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "7zip-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/{archive_name}")),
            sha256: None,
            archive_binary: Some("7zz".to_string()),
            binary_name: Some("7zz".to_string()),
            destination: Some("7zz".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir.clone()),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Failed);
    assert_eq!(result.items[0].failure_code, Some(ExitCode::Install));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[tokio::test]
async fn apply_install_plan_installs_archive_tree_release_when_url_has_query() -> anyhow::Result<()>
{
    let archive_name = "demo-tree.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[
        (
            "demo-tree/bin/demo",
            b"#!/bin/sh\necho tree\n".as_slice(),
            0o755,
        ),
        ("demo-tree/LICENSE", b"license\n".as_slice(), 0o644),
    ])?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    routes.insert(
        format!("/asset/{archive_name}?download=1"),
        routes[&format!("/asset/{archive_name}")].clone(),
    );
    let handle = spawn_mock_http_server(listener, routes, 1);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-tree".to_string(),
            method: "archive_tree_release".to_string(),
            version: None,
            url: Some(format!("{base}/asset/{archive_name}?download=1")),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("tree".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let request = crate::ExecutionRequest {
        managed_dir: Some(managed_dir.clone()),
        ..Default::default()
    };

    let result = crate::apply_install_plan(&plan, &request).await?;
    assert_eq!(
        result.items[0].status,
        BootstrapStatus::Installed,
        "detail={:?}",
        result.items[0].detail
    );
    assert!(
        managed_dir
            .join("tree")
            .join("demo-tree")
            .join("bin")
            .join("demo")
            .exists()
    );
    assert!(
        managed_dir
            .join("tree")
            .join("demo-tree")
            .join("LICENSE")
            .exists()
    );

    handle.join().expect("mock server thread join");
    Ok(())
}

#[test]
fn install_binary_from_tar_xz_uses_hint() -> anyhow::Result<()> {
    let archive = make_tar_xz_archive(&[(
        "node-v1.0.0-linux-x64/bin/node",
        b"mock-node".as_slice(),
        0o755,
    )])?;
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("node");
    install_binary_from_archive(
        "node-v1.0.0-linux-x64.tar.xz",
        &archive,
        "node",
        "node",
        &destination,
        Some("node-v1.0.0-linux-x64/bin/node"),
    )?;
    let content = std::fs::read_to_string(&destination)?;
    assert_eq!(content, "mock-node");
    Ok(())
}

#[test]
fn extract_archive_tree_preserves_existing_destination_on_invalid_archive() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("tree");
    std::fs::create_dir_all(&destination)?;
    std::fs::write(destination.join("old.txt"), "stale")?;

    install_archive_tree_from_bytes("demo.zip", b"not-a-zip", &destination)
        .expect_err("invalid archive should fail");
    assert_eq!(
        std::fs::read_to_string(destination.join("old.txt"))?,
        "stale"
    );
    Ok(())
}

#[test]
fn extract_archive_tree_replaces_existing_destination_after_successful_stage() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    let destination = tmp.path().join("tree");
    std::fs::create_dir_all(&destination)?;
    std::fs::write(destination.join("old.txt"), "stale")?;
    let archive = make_zip_archive(&[
        ("bin/demo", b"#!/bin/sh\necho demo\n".as_slice(), 0o755),
        ("LICENSE", b"demo-license\n".as_slice(), 0o644),
    ])?;

    install_archive_tree_from_bytes("demo.zip", &archive, &destination)?;

    assert!(!destination.join("old.txt").exists());
    assert!(destination.join("bin/demo").exists());
    assert!(destination.join("LICENSE").exists());
    Ok(())
}

#[test]
fn normalize_requested_tools_dedups_and_trims() {
    let tools = normalize_requested_tools(&[
        " git ".to_string(),
        "gh".to_string(),
        "git".to_string(),
        "  ".to_string(),
        "GH".to_string(),
    ]);
    assert_eq!(tools, vec!["git".to_string(), "gh".to_string()]);
}

#[test]
fn parse_sha256_user_input_rejects_short_value() {
    assert!(parse_sha256_user_input("abc").is_none());
}

#[test]
fn make_gateway_asset_candidate_normalizes_base_trailing_slash() {
    let out = make_gateway_asset_candidate(
        "https://gw.example/",
        "git",
        "v1.2.3",
        "MinGit-1.2.3-64-bit.zip",
    );
    assert_eq!(
        out,
        "https://gw.example/toolchain/git/v1.2.3/MinGit-1.2.3-64-bit.zip"
    );
}

#[test]
fn gateway_candidate_for_git_release_download_url_rejects_non_matching_or_embedded_paths() {
    let cfg = InstallerRuntimeConfig {
        gateway: GatewayRoutingPolicy {
            base: Some("https://gw.example".to_string()),
            country: Some("CN".to_string()),
        },
        ..test_runtime_config()
    };
    assert!(
        gateway_candidate_for_git_release_download_url(
            &cfg,
            "https://example.com/download/v1/file.zip"
        )
        .is_none()
    );
    assert!(
        gateway_candidate_for_git_release_download_url(
            &cfg,
            "https://mirror.example/github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip"
        )
        .is_none()
    );
    assert!(
        gateway_candidate_for_git_release_download_url(
            &cfg,
            "http://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit.zip"
        )
        .is_none()
    );
    assert!(
        gateway_candidate_for_git_release_download_url(
            &cfg,
            "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/nested/MinGit.zip"
        )
        .is_none()
    );
}

#[test]
fn system_recipes_cover_macos() {
    let package = SystemPackageName::new("git").expect("valid package name");
    let recipes = default_system_package_install_recipes_for_os("macos", &package);
    assert_eq!(recipes.len(), 1);
    assert_eq!(recipes[0].program, "brew");
}

#[test]
fn target_binary_ext_matches_windows_and_unix() {
    assert_eq!(
        executable_suffix_for_target("x86_64-pc-windows-msvc"),
        ".exe"
    );
    assert_eq!(executable_suffix_for_target("x86_64-unknown-linux-gnu"), "");
}

#[test]
fn resolve_target_triple_accepts_supported_trimmed_override() {
    let detected = resolve_target_triple(
        Some("  x86_64-pc-windows-msvc  "),
        "x86_64-unknown-linux-gnu",
    );
    assert_eq!(detected, "x86_64-pc-windows-msvc".to_string());
}

#[test]
fn resolve_target_triple_rejects_unknown_trimmed_override() {
    let detected = resolve_target_triple(Some("  custom-target  "), "x86_64-unknown-linux-gnu");
    assert_eq!(detected, "custom-target".to_string());
}

#[test]
fn resolve_managed_toolchain_dir_uses_override() {
    let path = PathBuf::from("/tmp/toolchain-test");
    let out = resolve_managed_toolchain_dir(Some(path.as_path()), "x86_64-unknown-linux-gnu")
        .expect("resolved");
    assert_eq!(out, path);
}

#[test]
fn default_managed_dir_under_data_root_uses_omne_layout() {
    let out = default_managed_dir_under_data_root(
        Path::new("/home/test/.omne_data"),
        "x86_64-unknown-linux-gnu",
    );
    assert_eq!(
        out,
        PathBuf::from("/home/test/.omne_data/toolchain/x86_64-unknown-linux-gnu/bin")
    );
}

#[test]
fn validate_plan_rejects_unknown_schema_version() {
    let plan = InstallPlan {
        schema_version: Some(999),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "unknown".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("schema version should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_empty_items() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: Vec::new(),
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("empty plan should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_unknown_method() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "unknown".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("unknown method should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_cross_target_for_host_bound_method() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "pip".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("demo".to_string()),
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(&plan, "x86_64-unknown-linux-gnu", "aarch64-apple-darwin")
        .expect_err("cross-target pip should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_parent_components_in_destination() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("../escape".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("parent components should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_absolute_destination() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("/tmp/escape".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("absolute destination should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_windows_absolute_destination_on_non_windows_host() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("C:\\tools\\demo.exe".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };

    let err = validate_plan(&plan, "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc")
        .expect_err("non-windows host should reject windows absolute destination");
    assert_eq!(err.exit_code(), ExitCode::Usage);
    assert!(err.to_string().contains("cannot be an absolute path"));
}

#[test]
fn validate_plan_resolves_workspace_package_destination_against_plan_base_dir() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "workspace-react".to_string(),
            method: "workspace_package".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("apps/demo-web".to_string()),
            package: Some("react".to_string()),
            manager: None,
            python: None,
        }],
    };
    let plan_base_dir = Path::new("/tmp/install-plans/demo");

    let resolved = validate_plan_with_base_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        plan_base_dir,
    )
    .expect("workspace destination should resolve");

    match &resolved[0] {
        ResolvedPlanItem::WorkspacePackage(item) => {
            assert_eq!(item.destination, plan_base_dir.join("apps/demo-web"));
        }
        other => panic!("unexpected resolved item: {other:?}"),
    }
}

#[test]
fn validate_plan_rejects_duplicate_item_ids() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-a.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: None,
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-b.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: None,
                package: None,
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("duplicate item ids should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "demo-a".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-a.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("bin/shared-demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "demo-b".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-b.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("bin/shared-demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain/bin"),
    )
    .expect_err("destination conflicts should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_parent_child_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "archive-demo".to_string(),
                method: "archive_tree_release".to_string(),
                version: None,
                url: Some("https://example.com/demo.zip".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("tools".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "release-demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("tools/demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain/bin"),
    )
    .expect_err("parent-child destination conflicts should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_equivalent_destinations_after_normalization() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "demo-a".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-a.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("./bin/shared-demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "demo-b".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-b.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("bin/shared-demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain/bin"),
    )
    .expect_err("normalized destination conflicts should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_npm_global_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "release-demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("http-server".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "npm-demo".to_string(),
                method: "npm_global".to_string(),
                version: None,
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: Some("http-server".to_string()),
                destination: None,
                package: Some("http-server@14.1.1".to_string()),
                manager: Some("pnpm".to_string()),
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain/bin"),
    )
    .expect_err("npm_global conflict should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_windows_pnpm_global_cmd_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "release-demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo.zip".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("http-server.cmd".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "pnpm-demo".to_string(),
                method: "npm_global".to_string(),
                version: None,
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: Some("http-server".to_string()),
                destination: None,
                package: Some("http-server@14.1.1".to_string()),
                manager: Some("pnpm".to_string()),
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
        Path::new(r"C:\toolchain"),
    )
    .expect_err("pnpm on Windows should reserve the .cmd entrypoint");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_windows_bun_global_cmd_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "release-demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo.zip".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("bin/http-server.cmd".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "bun-demo".to_string(),
                method: "npm_global".to_string(),
                version: None,
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: Some("http-server".to_string()),
                destination: None,
                package: Some("http-server@14.1.1".to_string()),
                manager: Some("bun".to_string()),
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
        Path::new(r"C:\toolchain"),
    )
    .expect_err("bun on Windows should reserve the .cmd entrypoint under bin/");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_go_install_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "release-demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: Some("hello".to_string()),
                destination: None,
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "go-demo".to_string(),
                method: "go_install".to_string(),
                version: None,
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: Some("hello".to_string()),
                destination: None,
                package: Some("example.com/hello@v1.0.0".to_string()),
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain"),
    )
    .expect_err("go_install conflict should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_windows_case_folded_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "demo-a".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-a.zip".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("Foo.exe".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "demo-b".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo-b.zip".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("foo.exe".to_string()),
                package: None,
                manager: None,
                python: None,
            },
        ],
    };

    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        Path::new("/tmp/toolchain"),
    )
    .expect_err("windows targets should treat case-folded destinations as conflicts");
    assert_eq!(err.exit_code(), ExitCode::Usage);
    assert!(err.to_string().contains("same destination"));
}

#[test]
fn validate_plan_rejects_windows_absolute_destination_for_release() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.zip".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some(r"C:\tools\demo.exe".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };

    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
        Path::new(r"C:\managed"),
    )
    .expect_err("windows absolute destination should be rejected for managed release items");
    assert_eq!(err.exit_code(), ExitCode::Usage);
    assert!(err.to_string().contains("cannot be an absolute path"));
}

#[test]
fn validate_plan_rejects_nested_destination_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "sdk-tree".to_string(),
                method: "archive_tree_release".to_string(),
                version: None,
                url: Some("https://example.com/sdk.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("sdk".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "sdk-launcher".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/sdk-launcher.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("sdk/bin/demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain/bin"),
    )
    .expect_err("nested destination conflicts should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
    assert!(err.to_string().contains("overlapping destinations"));
}

#[test]
fn validate_plan_rejects_uv_python_install_root_conflicts() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "python-demo".to_string(),
                method: "uv_python".to_string(),
                version: Some("3.13.12".to_string()),
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: None,
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "archive-demo".to_string(),
                method: "archive_tree_release".to_string(),
                version: None,
                url: Some("https://example.com/demo.zip".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some(".uv-python".to_string()),
                package: None,
                manager: None,
                python: None,
            },
        ],
    };
    let err = validate_plan_with_managed_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        Path::new("/tmp/toolchain/bin"),
    )
    .expect_err("uv_python install root conflict should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_non_archive_tree_asset_during_structure_validation() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "archive-demo".to_string(),
            method: "archive_tree_release".to_string(),
            version: None,
            url: Some("https://example.com/demo.bin?download=1".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("non-archive asset should be rejected before execution");
    assert_eq!(err.exit_code(), ExitCode::Usage);
    assert!(err.to_string().contains("supported archive asset"));
}

#[test]
fn public_validate_install_plan_stays_structure_only_without_managed_dir_context() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![
            InstallPlanItem {
                id: "release-demo".to_string(),
                method: "release".to_string(),
                version: None,
                url: Some("https://example.com/demo.tar.gz".to_string()),
                sha256: None,
                archive_binary: None,
                binary_name: None,
                destination: Some("demo".to_string()),
                package: None,
                manager: None,
                python: None,
            },
            InstallPlanItem {
                id: "cargo-demo".to_string(),
                method: "cargo_install".to_string(),
                version: None,
                url: None,
                sha256: None,
                archive_binary: None,
                binary_name: Some("demo".to_string()),
                destination: None,
                package: Some("demo".to_string()),
                manager: None,
                python: None,
            },
        ],
    };

    crate::validate_install_plan(&plan, None)
        .expect("public validator should not guess managed_dir-dependent conflicts");
}

#[test]
fn validate_plan_resolves_cargo_install_local_paths_against_plan_base_dir() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "cargo-demo".to_string(),
            method: "cargo_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("tools/cargo-demo".to_string()),
            manager: None,
            python: None,
        }],
    };
    let plan_base_dir = Path::new("/tmp/install-plans/demo");

    let resolved = validate_plan_with_base_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        plan_base_dir,
    )
    .expect("cargo local path should resolve");

    match &resolved[0] {
        crate::plan_items::ResolvedPlanItem::CargoInstall(item) => {
            assert_eq!(
                item.source,
                crate::plan_items::CargoInstallSource::LocalPath(
                    plan_base_dir.join("tools/cargo-demo")
                )
            );
        }
        other => panic!("unexpected resolved item: {other:?}"),
    }
}

#[test]
fn validate_plan_resolves_go_install_local_paths_against_plan_base_dir() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "go-demo".to_string(),
            method: "go_install".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("./cmd/demo".to_string()),
            manager: None,
            python: None,
        }],
    };
    let plan_base_dir = Path::new("/tmp/install-plans/demo");

    let resolved = validate_plan_with_base_dir(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        plan_base_dir,
    )
    .expect("go local path should resolve");

    match &resolved[0] {
        crate::plan_items::ResolvedPlanItem::GoInstall(item) => {
            assert_eq!(
                item.source,
                crate::plan_items::GoInstallSource::LocalPath(plan_base_dir.join("./cmd/demo"))
            );
        }
        other => panic!("unexpected resolved item: {other:?}"),
    }
}

#[test]
fn validate_plan_rejects_non_http_release_url() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("file:///tmp/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: None,
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("non-http url should be rejected");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_release_with_package_field() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.tar.gz".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("demo".to_string()),
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("release should reject package field");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_pip_destination_field() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "pip".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: Some("tmp/demo".to_string()),
            package: Some("demo".to_string()),
            manager: None,
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("pip should reject destination field");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[test]
fn validate_plan_rejects_apt_with_non_apt_manager() {
    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo".to_string(),
            method: "apt".to_string(),
            version: None,
            url: None,
            sha256: None,
            archive_binary: None,
            binary_name: None,
            destination: None,
            package: Some("demo".to_string()),
            manager: Some("dnf".to_string()),
            python: None,
        }],
    };
    let err = validate_plan(
        &plan,
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    )
    .expect_err("apt should reject non-apt manager");
    assert_eq!(err.exit_code(), ExitCode::Usage);
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_falls_back_to_python_mirror() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  if [ -z "$UV_PYTHON_INSTALL_MIRROR" ]; then
    echo "official source failed" >&2
    exit 1
  fi
  mkdir -p "$UV_PYTHON_BIN_DIR"
  cat > "$UV_PYTHON_BIN_DIR/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "$UV_PYTHON_BIN_DIR/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13.12".to_string(),
        version: "3.13.12".to_string(),
    };
    let redacted_mirror = "https://mirror.example/python";
    let cfg = InstallerRuntimeConfig {
        python_mirrors: PythonMirrorPolicy {
            install_mirrors: vec![
                "https://user:secret@mirror.example/python?token=abc".to_string(),
            ],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;
    assert_eq!(result.status, BootstrapStatus::Installed);
    assert_eq!(
        result.source.as_deref(),
        Some(format!("python-mirror:{redacted_mirror}").as_str())
    );
    assert_eq!(result.source_kind, Some(BootstrapSourceKind::PythonMirror));
    assert_eq!(
        result.destination.as_deref(),
        Some(
            managed_dir
                .join("python3.13")
                .display()
                .to_string()
                .as_str()
        )
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_ignores_inherited_uv_environment() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  if [ "$UV_PYTHON_INSTALL_MIRROR" = "https://host.example/python" ]; then
    echo "inherited mirror leaked" >&2
    exit 17
  fi
  mkdir -p "$UV_PYTHON_BIN_DIR"
  cat > "$UV_PYTHON_BIN_DIR/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "$UV_PYTHON_BIN_DIR/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let result = std::process::Command::new(std::env::current_exe()?)
        .env(
            "TOOLCHAIN_TEST_MANAGED_DIR",
            managed_dir.display().to_string(),
        )
        .env("UV_PYTHON_INSTALL_MIRROR", "https://host.example/python")
        .env("RUST_TEST_THREADS", "1")
        .arg("--exact")
        .arg("execute_uv_python_item_ignores_inherited_uv_environment_helper")
        .output()?;
    if !result.status.success() {
        anyhow::bail!(
            "helper test failed: status={} stderr={} stdout={}",
            result.status,
            String::from_utf8_lossy(&result.stderr),
            String::from_utf8_lossy(&result.stdout)
        );
    }
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_ignores_inherited_uv_environment_helper() -> anyhow::Result<()> {
    let Some(managed_dir) = std::env::var_os("TOOLCHAIN_TEST_MANAGED_DIR").map(PathBuf::from)
    else {
        return Ok(());
    };

    let item = UvPythonPlanItem {
        id: "python3.13.12".to_string(),
        version: "3.13.12".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await?;

    assert_eq!(result.status, BootstrapStatus::Installed);
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[test]
fn execute_uv_python_item_reuses_healthy_host_uv_when_managed_uv_is_missing() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    let bin_dir = tmp.path().join("bin");
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &bin_dir.join("uv"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  mkdir -p "{}"
  cat > "{}/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "{}/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
            managed_dir.display(),
            managed_dir.display(),
            managed_dir.display()
        ),
    )?;

    with_path_prepend(&bin_dir, || -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let item = UvPythonPlanItem {
                id: "python3.13.12".to_string(),
                version: "3.13.12".to_string(),
            };
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?;

            let result = execute_uv_python_item(
                &item,
                "x86_64-unknown-linux-gnu",
                &managed_dir,
                &test_runtime_config(),
                &client,
            )
            .await?;

            assert_eq!(result.status, BootstrapStatus::Installed);
            assert_eq!(result.source_kind, Some(BootstrapSourceKind::Canonical));
            assert!(
                result
                    .detail
                    .as_deref()
                    .unwrap_or_default()
                    .contains("using healthy host uv")
            );
            assert!(!managed_dir.join("uv").exists());
            assert!(managed_dir.join("python3.13").exists());
            Ok(())
        })
    })?;
    Ok(())
}

#[cfg_attr(windows, ignore = "mock python shim is unix-specific")]
#[test]
fn find_managed_python_executable_requires_exact_patch_version() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("python3.13"),
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;

    let found = find_managed_python_executable(&managed_dir, "3.13.1", "x86_64-unknown-linux-gnu");
    assert!(found.is_none(), "3.13.12 must not satisfy 3.13.1");
    Ok(())
}

#[cfg(unix)]
#[test]
fn find_managed_python_executable_skips_unreadable_installation_subdirectory() -> anyhow::Result<()>
{
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let install_root = managed_python_installation_dir(&managed_dir);
    let blocked_dir = install_root.join("000-blocked");
    let interpreter = install_root.join("999-good").join("bin").join("python3.13");
    std::fs::create_dir_all(blocked_dir.join("bin"))?;
    std::fs::create_dir_all(interpreter.parent().expect("python parent"))?;
    write_executable(
        &interpreter,
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;

    std::fs::set_permissions(&blocked_dir, std::fs::Permissions::from_mode(0o000))?;
    let discovered =
        find_managed_python_executable(&managed_dir, "3.13.12", "x86_64-unknown-linux-gnu");
    std::fs::set_permissions(&blocked_dir, std::fs::Permissions::from_mode(0o755))?;

    assert_eq!(discovered, Some(interpreter));
    Ok(())
}

#[cfg_attr(windows, ignore = "mock python shim is unix-specific")]
#[test]
fn find_managed_python_executable_accepts_latest_patch_for_family_version() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("python3.13"),
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;

    let found = find_managed_python_executable(&managed_dir, "3.13", "x86_64-unknown-linux-gnu");
    assert_eq!(found, Some(managed_dir.join("python3.13")));
    Ok(())
}

#[cfg_attr(windows, ignore = "mock python shim is unix-specific")]
#[test]
fn find_managed_python_executable_prefers_highest_patch_with_family_selector() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let install_root = managed_python_installation_dir(&managed_dir);
    let older = install_root.join("cpython-3.13.1-linux-x86_64-gnu/bin/python3.13");
    let newer = install_root.join("cpython-3.13.12-linux-x86_64-gnu/bin/python3.13");
    std::fs::create_dir_all(older.parent().expect("older parent"))?;
    std::fs::create_dir_all(newer.parent().expect("newer parent"))?;
    write_executable(
        &older,
        r#"#!/bin/sh
echo "Python 3.13.1"
"#,
    )?;
    write_executable(
        &newer,
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;

    let found = find_managed_python_executable(&managed_dir, "3.13", "x86_64-unknown-linux-gnu");
    assert_eq!(found, Some(newer));
    Ok(())
}

#[cfg_attr(windows, ignore = "mock python shim is unix-specific")]
#[test]
fn find_managed_python_executable_accepts_major_only_selector() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("python3.13"),
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;

    let found = find_managed_python_executable(&managed_dir, "3", "x86_64-unknown-linux-gnu");
    assert_eq!(found, Some(managed_dir.join("python3.13")));
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_returns_real_interpreter_from_installation_dir()
-> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  install_root="$UV_PYTHON_INSTALL_DIR/cpython-3.13.12-linux-x86_64-gnu/bin"
  mkdir -p "$install_root"
  cat > "$install_root/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "$install_root/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13.12".to_string(),
        version: "3.13.12".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await?;
    assert_eq!(result.status, BootstrapStatus::Installed);
    let destination = PathBuf::from(result.destination.expect("python destination"));
    assert!(destination.exists(), "expected managed python to exist");
    let version_output = std::process::Command::new(&destination)
        .arg("--version")
        .output()?;
    let stdout = String::from_utf8_lossy(&version_output.stdout);
    let stderr = String::from_utf8_lossy(&version_output.stderr);
    assert!(
        stdout.contains("Python 3.13.12") || stderr.contains("Python 3.13.12"),
        "unexpected python version output: stdout={stdout:?} stderr={stderr:?}"
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_accepts_major_only_selector() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  install_root="$UV_PYTHON_INSTALL_DIR/cpython-3.13.12-linux-x86_64-gnu/bin"
  mkdir -p "$install_root"
  cat > "$install_root/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "$install_root/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3".to_string(),
        version: "3".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await?;
    assert_eq!(
        result.destination.as_deref(),
        Some(
            managed_python_installation_dir(&managed_dir)
                .join("cpython-3.13.12-linux-x86_64-gnu")
                .join("bin")
                .join("python3.13")
                .display()
                .to_string()
                .as_str()
        )
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_prefers_newly_installed_family_version() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let install_root = managed_python_installation_dir(&managed_dir);
    let older = install_root.join("cpython-3.13.1-linux-x86_64-gnu/bin/python3.13");
    std::fs::create_dir_all(older.parent().expect("older parent"))?;
    write_executable(
        &older,
        r#"#!/bin/sh
echo "Python 3.13.1"
"#,
    )?;
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  install_root="$UV_PYTHON_INSTALL_DIR/cpython-3.13.12-linux-x86_64-gnu/bin"
  mkdir -p "$install_root"
  cat > "$install_root/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "$install_root/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13".to_string(),
        version: "3.13".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await?;
    assert_eq!(
        result.destination.as_deref(),
        Some(
            managed_python_installation_dir(&managed_dir)
                .join("cpython-3.13.12-linux-x86_64-gnu")
                .join("bin")
                .join("python3.13")
                .display()
                .to_string()
                .as_str()
        )
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_accepts_updated_existing_interpreter() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("python3.13"),
        r#"#!/bin/sh
echo "Python 3.13.1"
"#,
    )?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  cat > "$UV_PYTHON_BIN_DIR/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.12"
EOF
  chmod +x "$UV_PYTHON_BIN_DIR/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13".to_string(),
        version: "3.13".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await?;
    assert_eq!(
        result.destination.as_deref(),
        Some(
            managed_dir
                .join("python3.13")
                .display()
                .to_string()
                .as_str()
        )
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_requires_exact_patch_match() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("python3.13"),
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  install_root="$UV_PYTHON_INSTALL_DIR/cpython-3.13.1-linux-x86_64-gnu/bin"
  mkdir -p "$install_root"
  cat > "$install_root/python3.13" <<'EOF'
#!/bin/sh
echo "Python 3.13.1"
EOF
  chmod +x "$install_root/python3.13"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13.1".to_string(),
        version: "3.13.1".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await?;
    assert_eq!(
        result.destination.as_deref(),
        Some(
            managed_python_installation_dir(&managed_dir)
                .join("cpython-3.13.1-linux-x86_64-gnu")
                .join("bin")
                .join("python3.13")
                .display()
                .to_string()
                .as_str()
        )
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_rejects_stale_matching_interpreter_when_install_creates_nothing()
-> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("python3.13"),
        r#"#!/bin/sh
echo "Python 3.13.12"
"#,
    )?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  mkdir -p "$UV_PYTHON_INSTALL_DIR"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13.12".to_string(),
        version: "3.13.12".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let err = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await
    .expect_err("stale interpreter should not satisfy install");
    assert_eq!(err.exit_code(), ExitCode::Install);
    assert!(err.detail().contains(
        "no newly created or updated managed Python executable matching `3.13.12` was found"
    ));
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_python_item_fails_when_no_matching_interpreter_is_created() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  mkdir -p "$UV_PYTHON_INSTALL_DIR"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let item = UvPythonPlanItem {
        id: "python3.13.12".to_string(),
        version: "3.13.12".to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let err = execute_uv_python_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &test_runtime_config(),
        &client,
    )
    .await
    .expect_err("missing interpreter should fail");
    assert_eq!(err.exit_code(), ExitCode::Install);
    assert!(err.detail().contains(
        "no newly created or updated managed Python executable matching `3.13.12` was found"
    ));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn apply_install_plan_rejects_destination_escape_via_symlink() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&managed_dir)?;
    std::fs::create_dir_all(&outside)?;
    symlink(&outside, managed_dir.join("escape"))?;

    let plan = InstallPlan {
        schema_version: Some(PLAN_SCHEMA_VERSION),
        items: vec![InstallPlanItem {
            id: "demo-release".to_string(),
            method: "release".to_string(),
            version: None,
            url: Some("https://example.com/demo.bin".to_string()),
            sha256: None,
            archive_binary: None,
            binary_name: Some("demo".to_string()),
            destination: Some("escape/demo".to_string()),
            package: None,
            manager: None,
            python: None,
        }],
    };

    let result = crate::apply_install_plan(
        &plan,
        &ExecutionRequest {
            managed_dir: Some(managed_dir.clone()),
            ..ExecutionRequest::default()
        },
    )
    .await?;
    assert_eq!(result.items[0].status, BootstrapStatus::Failed);
    assert!(
        result.items[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("escapes via symlink component")
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_prefers_reachable_backup_index() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let log_path = managed_dir.join("index.log");
    write_executable(
        &managed_dir.join("uv"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  echo "$UV_DEFAULT_INDEX" > "{}"
  cat > "$UV_TOOL_BIN_DIR/ruff" <<'EOF'
#!/bin/sh
echo "ruff 0.0.0"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
            log_path.display()
        ),
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let backup_index = format!("http://user:secret@{addr}/mirror/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/mirror/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 2);

    let item = UvToolPlanItem {
        id: "ruff".to_string(),
        package: "ruff".to_string(),
        python: Some("3.13.12".to_string()),
        binary_name: "ruff".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![format!("{base}/official/simple"), backup_index.clone()],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;
    let redacted_backup_index = format!("{base}/mirror/simple");
    assert_eq!(
        result.source.as_deref(),
        Some(format!("package-index:{redacted_backup_index}").as_str())
    );
    assert_eq!(result.source_kind, Some(BootstrapSourceKind::PackageIndex));
    let used_index = std::fs::read_to_string(&log_path)?;
    assert_eq!(used_index.trim(), backup_index);

    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv/python shims are unix-specific")]
#[test]
fn execute_uv_tool_item_bootstraps_reusable_uv_from_package_index() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let bin_dir = tmp.path().join("bin");
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::create_dir_all(&managed_dir)?;
    let pip_log = managed_dir.join("pip-bootstrap.log");
    let uv_log = managed_dir.join("uv-tool.log");

    write_executable(
        &bin_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  exit 42
fi
exit 42
"#,
    )?;
    write_executable(
        &bin_dir.join("python3"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "-m" ] && [ "$2" = "pip" ] && [ "$3" = "install" ]; then
  target=""
  index=""
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--target" ]; then
      target="$2"
      shift 2
      continue
    fi
    if [ "$1" = "--index-url" ]; then
      index="$2"
      shift 2
      continue
    fi
    shift
  done
  mkdir -p "$target"
  printf '%s\n' "$index" > "{}"
  exit 0
fi
if [ "$1" = "-m" ] && [ "$2" = "uv" ] && [ "$3" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "-m" ] && [ "$2" = "uv" ] && [ "$3" = "tool" ] && [ "$4" = "install" ]; then
  printf '%s\n' "$UV_DEFAULT_INDEX" > "{}"
  mkdir -p "$UV_TOOL_BIN_DIR"
  cat > "$UV_TOOL_BIN_DIR/ruff" <<'EOF'
#!/bin/sh
echo "ruff 0.1.0"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
            pip_log.display(),
            uv_log.display()
        ),
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    with_path_prepend(&bin_dir, || -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let item = UvToolPlanItem {
                id: "ruff".to_string(),
                package: "ruff".to_string(),
                python: None,
                binary_name: "ruff".to_string(),
                binary_name_explicit: false,
            };
            let cfg = InstallerRuntimeConfig {
                github_releases: GitHubReleasePolicy {
                    api_bases: vec!["http://127.0.0.1:9/api".to_string()],
                    token: None,
                },
                package_indexes: PackageIndexPolicy {
                    indexes: vec![index.clone()],
                },
                ..test_runtime_config()
            };
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?;

            let result = execute_uv_tool_item(
                &item,
                "x86_64-unknown-linux-gnu",
                &managed_dir,
                &cfg,
                &client,
            )
            .await?;

            assert_eq!(result.status, BootstrapStatus::Installed);
            assert_eq!(
                result.source.as_deref(),
                Some(format!("package-index:{index}").as_str())
            );
            assert!(
                result
                    .detail
                    .as_deref()
                    .unwrap_or_default()
                    .contains("bootstrapped reusable uv with `python3 -m pip`")
            );
            assert_eq!(std::fs::read_to_string(&pip_log)?.trim(), index);
            assert_eq!(std::fs::read_to_string(&uv_log)?.trim(), index);
            assert!(
                managed_dir
                    .join(".uv-bootstrap")
                    .join("bin")
                    .join("uv")
                    .exists()
            );
            assert!(managed_dir.join("ruff").exists());
            Ok(())
        })
    })?;

    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_ignores_inherited_uv_environment() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  if [ "$UV_DEFAULT_INDEX" = "https://host.example/simple" ]; then
    echo "inherited index leaked" >&2
    exit 17
  fi
  mkdir -p "$UV_TOOL_BIN_DIR"
  cat > "$UV_TOOL_BIN_DIR/ruff" <<'EOF'
#!/bin/sh
echo "ruff 0.1.0"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let result = std::process::Command::new(std::env::current_exe()?)
        .env(
            "TOOLCHAIN_TEST_MANAGED_DIR",
            managed_dir.display().to_string(),
        )
        .env("UV_DEFAULT_INDEX", "https://host.example/simple")
        .env("RUST_TEST_THREADS", "1")
        .arg("--exact")
        .arg("execute_uv_tool_item_ignores_inherited_uv_environment_helper")
        .output()?;
    if !result.status.success() {
        anyhow::bail!(
            "helper test failed: status={} stderr={} stdout={}",
            result.status,
            String::from_utf8_lossy(&result.stderr),
            String::from_utf8_lossy(&result.stdout)
        );
    }
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_ignores_inherited_uv_environment_helper() -> anyhow::Result<()> {
    let Some(managed_dir) = std::env::var_os("TOOLCHAIN_TEST_MANAGED_DIR").map(PathBuf::from)
    else {
        return Ok(());
    };

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff".to_string(),
        package: "ruff".to_string(),
        python: None,
        binary_name: "ruff".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index.clone()],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;

    assert_eq!(result.status, BootstrapStatus::Installed);
    assert_eq!(
        result.source.as_deref(),
        Some(format!("package-index:{index}").as_str())
    );
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_reinstalls_broken_managed_uv_before_install() -> anyhow::Result<()> {
    let archive_name = "uv-x86_64-unknown-linux-gnu.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "uv-x86_64-unknown-linux-gnu/uv",
        b"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo uv 0.11.0\n  exit 0\nfi\nif [ \"$1\" = \"tool\" ] && [ \"$2\" = \"install\" ]; then\n  mkdir -p \"$UV_TOOL_BIN_DIR\"\n  cat > \"$UV_TOOL_BIN_DIR/ruff\" <<'EOF'\n#!/bin/sh\necho \"ruff 0.1.0\"\nEOF\n  chmod +x \"$UV_TOOL_BIN_DIR/ruff\"\n  exit 0\nfi\necho \"unexpected args: $*\" >&2\nexit 2\n"
            .as_slice(),
        0o755,
    )])?;
    let digest = sha256_hex(&archive_bytes);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base = format!("http://{addr}");
    let release_body = serde_json::json!({
        "tag_name": "0.11.0",
        "assets": [{
            "name": archive_name,
            "browser_download_url": format!("{base}/asset/{archive_name}"),
            "digest": format!("sha256:{digest}")
        }]
    })
    .to_string()
    .into_bytes();
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        "/api/repos/astral-sh/uv/releases/latest".to_string(),
        release_body,
    );
    routes.insert(format!("/asset/{archive_name}"), archive_bytes);
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 3);

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
exit 2
"#,
    )?;

    let item = UvToolPlanItem {
        id: "ruff".to_string(),
        package: "ruff".to_string(),
        python: None,
        binary_name: "ruff".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        github_releases: GitHubReleasePolicy {
            api_bases: vec![format!("{base}/api")],
            token: None,
        },
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;
    assert_eq!(result.status, BootstrapStatus::Installed);
    assert!(
        result
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("reinstalled managed uv")
    );
    assert!(managed_uv_is_healthy(&managed_dir.join("uv")));

    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_uses_binary_name_override() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let log_path = managed_dir.join("index.log");
    write_executable(
        &managed_dir.join("uv"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  if [ "$3" != "--force" ] || [ "$4" != "--from" ] || [ "$5" != "ruff-lsp" ] || [ "$6" != "ruff-lsp" ]; then
    echo "unexpected tool install args: $*" >&2
    exit 3
  fi
  echo "$UV_DEFAULT_INDEX" > "{}"
  cat > "$UV_TOOL_BIN_DIR/ruff-lsp" <<'EOF'
#!/bin/sh
echo "ruff-lsp 0.1.0"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff-lsp"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
            log_path.display()
        ),
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: true,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index.clone()],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;
    assert_eq!(result.status, BootstrapStatus::Installed);
    assert_eq!(
        result.source.as_deref(),
        Some(format!("package-index:{index}").as_str())
    );
    assert_eq!(
        result.destination.as_deref(),
        Some(managed_dir.join("ruff-lsp").display().to_string().as_str())
    );
    assert_eq!(std::fs::read_to_string(&log_path)?.trim(), index);
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_rejects_missing_binary_after_install() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let err = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await
    .expect_err("missing managed binary should fail");
    assert!(err.to_string().contains("expected managed binary"));
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_rejects_broken_binary_after_install() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  cat > "$UV_TOOL_BIN_DIR/ruff-lsp" <<'EOF'
#!/bin/sh
if [ "$1" = "--version" ]; then
  exit 23
fi
exit 23
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff-lsp"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let err = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await
    .expect_err("broken managed binary should fail health check");
    assert!(err.to_string().contains("failed --version health check"));
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_restores_previous_binary_when_install_fails() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let destination = managed_dir.join("ruff-lsp");
    let stale_binary = "#!/bin/sh\necho stale-uv-tool\n";
    write_executable(&destination, stale_binary)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  exit 7
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let err = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await
    .expect_err("failing uv tool install should not reuse stale binary");

    assert!(err.to_string().contains("failed"));
    assert_eq!(std::fs::read_to_string(&destination)?, stale_binary);
    assert!(
        !destination
            .with_file_name("ruff-lsp.toolchain-installer-backup")
            .exists()
    );
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_restores_previous_binary_when_install_leaves_no_binary()
-> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let destination = managed_dir.join("ruff-lsp");
    let stale_binary = "#!/bin/sh\necho stale-uv-tool\n";
    write_executable(&destination, stale_binary)?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let err = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await
    .expect_err("missing uv tool binary should not reuse stale binary");

    assert!(err.to_string().contains("expected managed binary"));
    assert_eq!(std::fs::read_to_string(&destination)?, stale_binary);
    assert!(
        !destination
            .with_file_name("ruff-lsp.toolchain-installer-backup")
            .exists()
    );
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[tokio::test]
async fn execute_uv_tool_item_replaces_previous_directory_destination_and_cleans_backup()
-> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let destination = managed_dir.join("ruff-lsp");
    std::fs::create_dir_all(&destination)?;
    std::fs::write(destination.join("old"), "stale")?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  mkdir -p "$UV_TOOL_BIN_DIR"
  cat > "$UV_TOOL_BIN_DIR/ruff-lsp" <<'EOF'
#!/bin/sh
echo "uv-tool-installed"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff-lsp"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;

    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    let output = std::process::Command::new(&destination)
        .arg("--version")
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "uv-tool-installed"
    );
    assert!(
        !destination
            .with_file_name("ruff-lsp.toolchain-installer-backup")
            .exists()
    );
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg(all(unix, not(windows)))]
#[tokio::test]
async fn execute_uv_tool_item_recovers_from_stale_backup_before_reinstall() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let destination = managed_dir.join("ruff-lsp");
    write_executable(&destination, "#!/bin/sh\necho current-uv-tool\n")?;
    let stale_backup = destination.with_file_name("ruff-lsp.toolchain-installer-backup");
    std::fs::write(&stale_backup, "stale-backup")?;
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  mkdir -p "$UV_TOOL_BIN_DIR"
  cat > "$UV_TOOL_BIN_DIR/ruff-lsp" <<'EOF'
#!/bin/sh
echo "uv-tool-installed"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff-lsp"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;

    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    let output = std::process::Command::new(&destination)
        .arg("--version")
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "uv-tool-installed"
    );
    assert!(
        !stale_backup.exists(),
        "canonical backup should be consumed or removed during recovery"
    );
    let quarantined = std::fs::read_dir(&managed_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.contains(".toolchain-installer-backup.stale-"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        quarantined.len(),
        1,
        "stale canonical backup should be quarantined once"
    );
    handle.join().expect("mock server thread join");
    Ok(())
}

#[cfg(all(unix, not(windows)))]
#[tokio::test]
async fn execute_uv_tool_item_succeeds_when_backup_cleanup_fails_after_replacement()
-> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir()?;
    let managed_dir = tmp.path().join("managed");
    std::fs::create_dir_all(&managed_dir)?;
    let destination = managed_dir.join("ruff-lsp");
    write_executable(&destination, "#!/bin/sh\necho stale-uv-tool\n")?;
    write_executable(
        &managed_dir.join("uv"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  mkdir -p "$UV_TOOL_BIN_DIR"
  cat > "$UV_TOOL_BIN_DIR/ruff-lsp" <<'EOF'
#!/bin/sh
echo "uv-tool-installed"
EOF
  chmod +x "$UV_TOOL_BIN_DIR/ruff-lsp"
  chmod 0555 "{}"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 2
"#,
            managed_dir.display()
        ),
    )?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let item = UvToolPlanItem {
        id: "ruff-installer".to_string(),
        package: "ruff-lsp".to_string(),
        python: None,
        binary_name: "ruff-lsp".to_string(),
        binary_name_explicit: false,
    };
    let cfg = InstallerRuntimeConfig {
        package_indexes: PackageIndexPolicy {
            indexes: vec![index],
        },
        ..test_runtime_config()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = execute_uv_tool_item(
        &item,
        "x86_64-unknown-linux-gnu",
        &managed_dir,
        &cfg,
        &client,
    )
    .await?;

    let mut permissions = std::fs::metadata(&managed_dir)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&managed_dir, permissions)?;

    assert_eq!(
        result.destination.as_deref(),
        Some(destination.to_str().unwrap())
    );
    let output = std::process::Command::new(&destination)
        .arg("--version")
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "uv-tool-installed"
    );
    let backup_path = destination.with_file_name("ruff-lsp.toolchain-installer-backup");
    if result.detail.as_deref().is_some_and(|detail| {
        detail.contains("warning: cannot remove staged managed binary backup")
    }) {
        assert!(
            backup_path.exists(),
            "warning should explain a retained backup"
        );
    } else {
        assert!(
            !backup_path.exists(),
            "successful backup cleanup should not leave a retained backup"
        );
    }
    handle.join().expect("mock server thread join");
    Ok(())
}

fn sha256_hex(content: &[u8]) -> String {
    hash_sha256(content).to_string()
}

fn make_tar_gz_archive(entries: &[(&str, &[u8], u32)]) -> anyhow::Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, body, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, *path, &mut Cursor::new(*body))
            .with_context(|| format!("append tar entry {path}"))?;
    }
    let encoder = builder.into_inner().context("finalize tar builder")?;
    let archive = encoder.finish().context("finalize gzip stream")?;
    Ok(archive)
}

fn make_tar_xz_archive(entries: &[(&str, &[u8], u32)]) -> anyhow::Result<Vec<u8>> {
    let encoder = xz2::write::XzEncoder::new(Vec::new(), 6);
    let mut builder = tar::Builder::new(encoder);
    for (path, body, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, *path, &mut Cursor::new(*body))
            .with_context(|| format!("append tar.xz entry {path}"))?;
    }
    let encoder = builder.into_inner().context("finalize tar.xz builder")?;
    let archive = encoder.finish().context("finalize xz stream")?;
    Ok(archive)
}

fn make_zip_archive(entries: &[(&str, &[u8], u32)]) -> anyhow::Result<Vec<u8>> {
    use std::io::Write;

    let mut writer = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut writer);
        for (path, body, mode) in entries {
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(*mode);
            archive
                .start_file(*path, options)
                .with_context(|| format!("start zip entry {path}"))?;
            archive
                .write_all(body)
                .with_context(|| format!("write zip entry {path}"))?;
        }
        archive.finish().context("finish zip archive")?;
    }
    Ok(writer.into_inner())
}

fn write_executable(path: &Path, body: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("chmod {}", path.display()))?;
    }
    Ok(())
}

fn with_path_prepend<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let mut entries = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing));
    }
    with_path_entries(entries, f)
}

fn with_path_override<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    with_path_entries(vec![path.to_path_buf()], f)
}

fn with_path_entries<T>(entries: Vec<PathBuf>, f: impl FnOnce() -> T) -> T {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("lock env guard");
    let original = std::env::var_os("PATH");
    let joined = std::env::join_paths(entries).expect("join PATH");
    let restore = EnvVarRestore::new("PATH", original);
    // SAFETY: tests hold a process-wide mutex while mutating PATH and restore it before unlock.
    unsafe {
        std::env::set_var("PATH", &joined);
    }
    let output = f();
    drop(restore);
    output
}

struct EnvVarRestore {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvVarRestore {
    fn new(key: &'static str, original: Option<OsString>) -> Self {
        Self { key, original }
    }
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => {
                // SAFETY: guarded by the same process-wide test mutex used by callers.
                unsafe {
                    std::env::set_var(self.key, value);
                }
            }
            None => {
                // SAFETY: guarded by the same process-wide test mutex used by callers.
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}

fn spawn_mock_http_server(
    listener: TcpListener,
    routes: HashMap<String, Vec<u8>>,
    expected_requests: usize,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for _ in 0..expected_requests {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buffer = [0_u8; 8192];
            let Ok(size) = stream.read(&mut buffer) else {
                continue;
            };
            if size == 0 {
                continue;
            }
            let request = String::from_utf8_lossy(&buffer[..size]);
            let request_line = request.lines().next().unwrap_or_default();
            let path = request_line.split_whitespace().nth(1).unwrap_or("/");
            let (status, body) = if let Some(body) = routes.get(path) {
                ("200 OK", body.clone())
            } else {
                ("404 Not Found", b"not found".to_vec())
            };
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    })
}

#[test]
fn gh_asset_suffix_matches_current_targets() {
    assert_eq!(
        gh_release_asset_suffix_for_target("x86_64-unknown-linux-gnu"),
        Some("_linux_amd64.tar.gz")
    );
}
