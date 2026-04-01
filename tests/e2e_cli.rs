use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;

fn bootstrap_cmd() -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.arg("bootstrap");
    cmd
}

fn path_with_prepend(path: &Path) -> OsString {
    let mut entries = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(entries).expect("join PATH")
}

#[test]
fn bootstrap_with_unknown_tool_returns_unsupported_status() {
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args(["--json", "--tool", "custom-tool"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["schema_version"], 1);
    assert!(json["host_triple"].as_str().unwrap_or_default().len() > 3);
    assert_eq!(json["items"][0]["tool"], "custom-tool");
    assert_eq!(json["items"][0]["status"], "unsupported");
}

#[cfg(unix)]
#[test]
fn bootstrap_unknown_tool_ignores_plain_path_file_and_stays_unsupported() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let plain = temp.path().join("demo-tool");
    std::fs::write(&plain, "not executable").expect("write plain file");
    let mut permissions = std::fs::metadata(&plain)
        .expect("stat plain file")
        .permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(&plain, permissions).expect("chmod plain file");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(temp.path()))
        .args(["--json", "--tool", "demo-tool"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["tool"], "demo-tool");
    assert_eq!(json["items"][0]["status"], "unsupported");
}

#[test]
fn direct_method_with_unknown_strategy_returns_usage_exit_code() {
    let mut cmd = bootstrap_cmd();
    cmd.args(["--json", "--method", "unknown", "--id", "demo"])
        .assert()
        .code(2);
}

#[test]
fn plan_file_rejects_unknown_method_without_network() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo", "method": "unknown" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--json", "--plan-file"])
        .arg(&plan_path)
        .assert()
        .code(2);
}

#[test]
fn method_and_plan_file_conflict_returns_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(&plan_path, r#"{"schema_version":1,"items":[]}"#).expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--method", "pip", "--id", "demo", "--plan-file"])
        .arg(&plan_path)
        .assert()
        .code(2);
}

#[test]
fn tool_and_method_conflict_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args(["--tool", "git", "--method", "pip", "--id", "demo"])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("`--tool` cannot be used with `--method`"));
}

#[test]
fn tool_and_plan_file_conflict_returns_usage_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo", "method": "uv" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args(["--tool", "git", "--plan-file"])
        .arg(&plan_path)
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("`--tool` cannot be used with `--plan-file`"));
}

#[test]
fn method_without_id_returns_failure() {
    let mut cmd = bootstrap_cmd();
    cmd.args(["--method", "pip"]).assert().code(2);
}

#[test]
fn bootstrap_mode_rejects_direct_plan_only_flags() {
    let mut cmd = bootstrap_cmd();
    cmd.args(["--package", "ruff"]).assert().code(2);
}

#[test]
fn plan_file_mode_rejects_direct_plan_only_flags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo", "method": "uv" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file"])
        .arg(&plan_path)
        .args(["--package", "ruff"])
        .assert()
        .code(2);
}

#[test]
fn missing_plan_file_returns_failure() {
    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file", "/tmp/not-exist-plan-file.json"])
        .assert()
        .code(2);
}

#[test]
fn invalid_plan_file_json_returns_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("broken-plan.json");
    std::fs::write(&plan_path, "{ invalid").expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn strict_mode_fails_when_item_failed() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--json",
        "--strict",
        "--method",
        "pip",
        "--id",
        "pip-missing-python",
        "--package",
        "demo-package",
        "--python",
        "/tmp/definitely-missing-python",
    ])
    .assert()
    .code(5);
}

#[test]
fn pip_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "pip",
            "--id",
            "pip-demo",
            "--package=--editable",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn npm_global_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "npm_global",
            "--id",
            "npm-demo",
            "--package=--workspace",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn workspace_package_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "workspace_package",
            "--id",
            "workspace-demo",
            "--package=--workspace",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn cargo_install_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "cargo_install",
            "--id",
            "cargo-demo",
            "--package=--git",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn go_install_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "go_install",
            "--id",
            "go-demo",
            "--package=--mod",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn rustup_component_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "rustup_component",
            "--id",
            "rustfmt-demo",
            "--package=--toolchain",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn uv_tool_option_like_package_returns_usage_error() {
    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args([
            "--method",
            "uv_tool",
            "--id",
            "uv-demo",
            "--package=--index-url",
        ])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("does not allow `package` to look like a command-line option"));
}

#[test]
fn strict_mode_unknown_method_still_returns_usage_exit_code() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--json",
        "--strict",
        "--method",
        "unknown",
        "--id",
        "unsupported-demo",
    ])
    .assert()
    .code(2);
}

#[test]
fn default_managed_dir_uses_home_omne_data_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_home = temp.path().join("home");
    std::fs::create_dir_all(&fake_home).expect("create fake home");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .args(["--json", "--tool", "custom-tool"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let target = json["target_triple"].as_str().expect("target triple");
    assert_eq!(
        json["managed_dir"],
        fake_home
            .join(".omne_data")
            .join("toolchain")
            .join(target)
            .join("bin")
            .display()
            .to_string()
    );
}

#[test]
fn omne_data_dir_env_overrides_home_default_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_home = temp.path().join("home");
    let omne_data_dir = temp.path().join("omne-root");
    std::fs::create_dir_all(&fake_home).expect("create fake home");
    std::fs::create_dir_all(&omne_data_dir).expect("create omne root");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .env("OMNE_DATA_DIR", &omne_data_dir)
        .args(["--json", "--tool", "custom-tool"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let target = json["target_triple"].as_str().expect("target triple");
    assert_eq!(
        json["managed_dir"],
        omne_data_dir
            .join("toolchain")
            .join(target)
            .join("bin")
            .display()
            .to_string()
    );
}

#[test]
fn default_bootstrap_json_shape_contains_target_and_items() {
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args(["--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert!(json["host_triple"].as_str().unwrap_or_default().len() > 3);
    assert!(json["target_triple"].as_str().unwrap_or_default().len() > 3);
    let items = json["items"].as_array().expect("items array");
    assert!(!items.is_empty());
}

#[test]
fn single_item_download_failure_uses_download_exit_code() {
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--method",
            "release",
            "--id",
            "demo-release",
            "--url",
            "http://127.0.0.1:9/demo.tar.gz",
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["error_code"], "download_failed");
}

#[test]
fn max_download_bytes_flag_limits_release_downloads() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/demo.bin".to_string(), b"0123456789".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--max-download-bytes",
            "4",
            "--method",
            "release",
            "--id",
            "demo-release",
            "--url",
            &format!("http://{addr}/demo.bin"),
            "--destination",
            "demo.bin",
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["error_code"], "download_failed");
    assert!(
        json["items"][0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("configured max download size 4")
    );

    handle.join().expect("mock server thread join");
}

fn non_host_target_triple() -> String {
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args(["--json", "--tool", "custom-tool"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let host = json["host_triple"].as_str().expect("host triple");
    [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    ]
    .into_iter()
    .find(|candidate| *candidate != host)
    .expect("non-host target triple candidate")
    .to_string()
}

#[test]
fn cross_target_host_bound_method_returns_usage_exit_code() {
    let target = non_host_target_triple();
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--json",
        "--target-triple",
        &target,
        "--method",
        "pip",
        "--id",
        "cross-target-pip",
        "--package",
        "demo-package",
    ])
    .assert()
    .code(2);
}

#[test]
fn bootstrap_rejects_cross_target_override() {
    let target = non_host_target_triple();
    let mut cmd = bootstrap_cmd();
    cmd.args(["--target-triple", &target, "--tool", "git"])
        .assert()
        .code(2);
}

#[test]
fn plan_file_rejects_unknown_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo", "method": "release", "uurl": "https://example.com/demo" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    let stderr = cmd
        .args(["--json", "--plan-file"])
        .arg(&plan_path)
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("unknown field `uurl`"));
}

#[test]
fn single_item_install_failure_uses_install_exit_code() {
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--method",
            "pip",
            "--id",
            "demo-pip",
            "--package",
            "demo-package",
            "--python",
            "/tmp/definitely-missing-python",
        ])
        .assert()
        .code(4)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
}

#[test]
fn unsupported_plan_schema_returns_usage_exit_code() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 999,
  "items": [
    { "id": "demo", "method": "unknown" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn relative_release_destination_is_resolved_under_managed_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "release",
            "--id",
            "demo-release",
            "--url",
            "http://127.0.0.1:9/demo.tar.gz",
            "--destination",
            "nested/demo-release",
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir
            .join("nested")
            .join("demo-release")
            .display()
            .to_string()
    );
}

#[test]
fn absolute_release_destination_returns_usage_exit_code() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--method",
        "release",
        "--id",
        "demo-release",
        "--url",
        "http://127.0.0.1:9/demo.tar.gz",
        "--destination",
        "/tmp/escape",
    ])
    .assert()
    .code(2);
}

#[cfg(not(windows))]
#[test]
fn windows_absolute_release_destination_is_rejected_on_non_windows_host() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--method",
        "release",
        "--target-triple",
        "x86_64-pc-windows-msvc",
        "--id",
        "demo-release",
        "--url",
        "http://127.0.0.1:9/demo.tar.gz",
        "--destination",
        r"C:\tools\demo.exe",
    ])
    .assert()
    .code(2);
}

#[test]
fn duplicate_plan_item_ids_return_usage_exit_code() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo", "method": "release", "url": "https://example.com/a.tar.gz" },
    { "id": "demo", "method": "release", "url": "https://example.com/b.tar.gz" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn conflicting_plan_destinations_return_usage_exit_code() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "demo-a", "method": "release", "url": "https://example.com/a.tar.gz", "destination": "bin/shared-demo" },
    { "id": "demo-b", "method": "release", "url": "https://example.com/b.tar.gz", "destination": "bin/shared-demo" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn archive_release_json_includes_archive_match() {
    let archive_name = "demo-release.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[(
        "demo-release/bin/demo",
        b"#!/bin/sh\necho archive-demo\n".as_slice(),
        0o755,
    )]);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "release",
            "--id",
            "demo-release",
            "--url",
            &format!("http://{addr}/{archive_name}"),
            "--binary-name",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["archive_match"]["format"], "tar_gz");
    assert_eq!(
        json["items"][0]["archive_match"]["path"],
        "demo-release/bin/demo"
    );

    handle.join().expect("mock server thread join");
}

#[test]
fn archive_tree_release_extracts_directory_tree() {
    let archive_name = "demo-tree.tar.gz";
    let archive_bytes = make_tar_gz_archive(&[
        (
            "demo-tree/bin/demo",
            b"#!/bin/sh\necho archive-tree-demo\n".as_slice(),
            0o755,
        ),
        ("demo-tree/LICENSE", b"demo-license\n".as_slice(), 0o644),
    ]);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let destination = managed_dir.join("tree");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "archive_tree_release",
            "--id",
            "demo-tree",
            "--url",
            &format!("http://{addr}/{archive_name}"),
            "--destination",
            "tree",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        destination.display().to_string()
    );
    assert!(destination.join("demo-tree/bin/demo").exists());
    assert!(destination.join("demo-tree/LICENSE").exists());

    handle.join().expect("mock server thread join");
}

#[cfg(unix)]
#[test]
fn archive_tree_release_extracts_tar_symlinks() {
    let archive_name = "demo-tree-symlink.tar.gz";
    let archive_bytes = make_tar_gz_archive_with_symlinks(
        &[
            (
                "demo-tree/lib/node_modules/npm/bin/npm-cli.js",
                b"console.log('npm')\n".as_slice(),
                0o755,
            ),
            ("demo-tree/LICENSE", b"demo-license\n".as_slice(), 0o644),
        ],
        &[(
            "demo-tree/bin/npm",
            "../lib/node_modules/npm/bin/npm-cli.js",
            0o755,
        )],
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let destination = managed_dir.join("tree");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "archive_tree_release",
            "--id",
            "demo-tree-symlink",
            "--url",
            &format!("http://{addr}/{archive_name}"),
            "--destination",
            "tree",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    let symlink_path = destination.join("demo-tree/bin/npm");
    assert!(symlink_path.exists());
    let metadata = std::fs::symlink_metadata(&symlink_path).expect("symlink metadata");
    assert!(metadata.file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&symlink_path).expect("read symlink"),
        Path::new("../lib/node_modules/npm/bin/npm-cli.js")
    );

    handle.join().expect("mock server thread join");
}

#[test]
fn archive_tree_release_extracts_zip_tree_without_top_level_directory() {
    let archive_name = "demo-tree.zip";
    let archive_bytes = make_zip_archive(&[
        ("bin/demo.exe", b"MZ".as_slice(), 0o755),
        ("LICENSE", b"demo-license\n".as_slice(), 0o644),
    ]);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/{archive_name}"), archive_bytes);
    let handle = spawn_mock_http_server(listener, routes, 1);

    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let destination = managed_dir.join("tree");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "archive_tree_release",
            "--id",
            "demo-tree-zip",
            "--url",
            &format!("http://{addr}/{archive_name}"),
            "--destination",
            "tree",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        destination.display().to_string()
    );
    assert!(destination.join("bin/demo.exe").exists());
    assert!(destination.join("LICENSE").exists());

    handle.join().expect("mock server thread join");
}

#[test]
fn archive_tree_release_retries_mirror_after_invalid_canonical_archive() {
    let archive_name = "demo-tree-retry.zip";
    let valid_archive = make_zip_archive(&[
        ("bin/demo.exe", b"MZ".as_slice(), 0o755),
        ("LICENSE", b"demo-license\n".as_slice(), 0o644),
    ]);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let base = format!("http://{addr}");
    let canonical_url = format!("{base}/{archive_name}");
    let mirror_prefix = format!("{base}/mirror/");
    let mirror_path = format!("/mirror/{canonical_url}");

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(format!("/{archive_name}"), b"not a zip archive".to_vec());
    routes.insert(mirror_path, valid_archive);
    let handle = spawn_mock_http_server(listener, routes, 2);

    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let destination = managed_dir.join("tree");
    std::fs::create_dir_all(&destination).expect("create destination");
    std::fs::write(destination.join("old.txt"), "stale").expect("write stale marker");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--mirror-prefix",
            &mirror_prefix,
            "--method",
            "archive_tree_release",
            "--id",
            "demo-tree-retry",
            "--url",
            &canonical_url,
            "--destination",
            "tree",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source_kind"], "mirror");
    assert!(!destination.join("old.txt").exists());
    assert!(destination.join("bin/demo.exe").exists());
    assert!(destination.join("LICENSE").exists());

    handle.join().expect("mock server thread join");
}

#[test]
fn archive_tree_release_allows_cross_target() {
    let target = non_host_target_triple();
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--json",
        "--target-triple",
        &target,
        "--method",
        "archive_tree_release",
        "--id",
        "cross-target-tree",
        "--url",
        "http://127.0.0.1:9/demo-tree.tar.gz",
    ])
    .assert()
    .code(3);
}

#[test]
fn pip_rejects_destination_field() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--method",
        "pip",
        "--id",
        "demo-pip",
        "--package",
        "demo-package",
        "--destination",
        "tmp/demo",
    ])
    .assert()
    .code(2);
}

#[test]
fn workspace_package_requires_destination_field() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--method",
        "workspace_package",
        "--id",
        "react",
        "--package",
        "react@18.3.1",
    ])
    .assert()
    .code(2);
}

#[cfg(unix)]
#[test]
fn workspace_package_accepts_absolute_destination() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
workspace=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--prefix" ]; then
    workspace="$2"
    shift 2
    continue
  fi
  shift
done
[ -n "$workspace" ] || exit 9
[ -f "$workspace/package.json" ] || exit 10
mkdir -p "$workspace/node_modules/react"
exit 0
"#,
    );

    let workspace_dir = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    std::fs::write(
        workspace_dir.join("package.json"),
        r#"{"name":"demo-workspace","private":true}"#,
    )
    .expect("write package.json");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--method",
            "workspace_package",
            "--id",
            "react",
            "--package",
            "react@18.3.1",
            "--destination",
        ])
        .arg(&workspace_dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source_kind"], "workspace_package");
    assert_eq!(
        json["items"][0]["destination"],
        workspace_dir.display().to_string()
    );
    assert!(workspace_dir.join("node_modules").join("react").exists());
}

#[cfg(unix)]
#[test]
fn workspace_package_plan_file_resolves_relative_destination_against_plan_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
workspace=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--prefix" ]; then
    workspace="$2"
    shift 2
    continue
  fi
  shift
done
[ -n "$workspace" ] || exit 9
[ -f "$workspace/package.json" ] || exit 10
/bin/mkdir -p "$workspace/node_modules/react"
exit 0
"#,
    );

    let managed_dir = temp.path().join("managed");
    let plan_dir = temp.path().join("plans");
    let workspace_dir = plan_dir.join("apps").join("demo-web");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    std::fs::write(
        workspace_dir.join("package.json"),
        r#"{"name":"demo-web","private":true}"#,
    )
    .expect("write package.json");

    let plan_path = plan_dir.join("workspace-plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    {
      "id": "react",
      "method": "workspace_package",
      "package": "react@18.3.1",
      "destination": "apps/demo-web"
    }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--plan-file",
        ])
        .arg(&plan_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        workspace_dir.display().to_string()
    );
    assert!(workspace_dir.join("node_modules").join("react").exists());
    assert!(!managed_dir.join("apps").join("demo-web").exists());
}

#[test]
fn npm_global_rejects_destination_field() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--method",
        "npm_global",
        "--id",
        "http-server",
        "--package",
        "http-server@14.1.1",
        "--destination",
        "tmp/http-server",
    ])
    .assert()
    .code(2);
}

#[cfg(unix)]
#[test]
fn npm_global_uses_custom_managed_dir_as_prefix_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
[ -n "$npm_config_prefix" ] || exit 9
mkdir -p "$npm_config_prefix/bin"
cat > "$npm_config_prefix/bin/http-server" <<'EOF'
#!/bin/sh
echo "14.1.1"
EOF
chmod +x "$npm_config_prefix/bin/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-npm-prefix");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir.join("bin").join("http-server");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn npm_global_falls_back_to_installed_package_binary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
[ -n "$npm_config_prefix" ] || exit 9
mkdir -p "$npm_config_prefix/lib/node_modules/http-server/bin"
cat > "$npm_config_prefix/lib/node_modules/http-server/package.json" <<'EOF'
{"name":"http-server","bin":{"http-server":"bin/http-server"}}
EOF
cat > "$npm_config_prefix/lib/node_modules/http-server/bin/http-server" <<'EOF'
#!/bin/sh
echo "14.1.1"
EOF
chmod +x "$npm_config_prefix/lib/node_modules/http-server/bin/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-npm-prefix");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir
        .join("lib")
        .join("node_modules")
        .join("http-server")
        .join("bin")
        .join("http-server");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn npm_global_does_not_report_success_from_unrelated_stale_binary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
[ -n "$npm_config_prefix" ] || exit 9
mkdir -p "$npm_config_prefix"
exit 0
"#,
    );

    let managed_dir = temp.path().join("custom-npm-prefix");
    let stale_binary = managed_dir.join("stale").join("http-server");
    if let Some(parent) = stale_binary.parent() {
        std::fs::create_dir_all(parent).expect("create stale parent");
    }
    write_executable(
        &stale_binary,
        r#"#!/bin/sh
echo "stale"
"#,
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--strict",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server-stale",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
        ])
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "failed");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
    assert_eq!(
        std::fs::read_to_string(&stale_binary).expect("read stale"),
        "#!/bin/sh\necho \"stale\"\n"
    );
}

#[cfg(unix)]
#[test]
fn npm_global_allows_leaf_symlink_destination_on_repeat_install() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
[ -n "$npm_config_prefix" ] || exit 9
package_dir="$npm_config_prefix/lib/node_modules/http-server"
mkdir -p "$package_dir/bin" "$npm_config_prefix/bin"
cat > "$package_dir/package.json" <<'EOF'
{"name":"http-server","bin":{"http-server":"bin/http-server"}}
EOF
cat > "$package_dir/bin/http-server" <<'EOF'
#!/bin/sh
echo "fresh $(date +%s%N)"
EOF
chmod +x "$package_dir/bin/http-server"
ln -sfn ../lib/node_modules/http-server/bin/http-server "$npm_config_prefix/bin/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-npm-prefix");
    let args = [
        "--json",
        "--managed-dir",
        managed_dir.to_str().expect("utf8 path"),
        "--method",
        "npm_global",
        "--id",
        "http-server",
        "--package",
        "http-server@14.1.1",
        "--binary-name",
        "http-server",
    ];

    let mut first = bootstrap_cmd();
    first
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args(args)
        .assert()
        .success();

    let mut second = bootstrap_cmd();
    let output = second
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir
            .join("bin")
            .join("http-server")
            .display()
            .to_string()
    );
}

#[cfg(unix)]
#[test]
fn npm_global_rejects_stale_manifest_binary_when_install_did_not_refresh_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
[ -n "$npm_config_prefix" ] || exit 9
package_dir="$npm_config_prefix/lib/node_modules/http-server"
mkdir -p "$package_dir/bin"
if [ ! -f "$package_dir/package.json" ]; then
  cat > "$package_dir/package.json" <<'EOF'
{"name":"http-server","bin":{"http-server":"bin/http-server"}}
EOF
fi
exit 0
"#,
    );

    let managed_dir = temp.path().join("custom-npm-prefix");
    let package_dir = managed_dir
        .join("lib")
        .join("node_modules")
        .join("http-server");
    std::fs::create_dir_all(package_dir.join("bin")).expect("create package dir");
    std::fs::write(
        package_dir.join("package.json"),
        r#"{"name":"http-server","bin":{"http-server":"bin/http-server"}}"#,
    )
    .expect("write manifest");
    write_executable(
        &package_dir.join("bin").join("http-server"),
        r#"#!/bin/sh
echo "stale"
"#,
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--strict",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server-manifest-stale",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
        ])
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "failed");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
}

#[cfg(unix)]
#[test]
fn npm_global_manifest_path_beats_nested_dependency_binary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_npm = fake_bin_dir.join("npm");
    write_executable(
        &fake_npm,
        r#"#!/bin/sh
[ -n "$npm_config_prefix" ] || exit 9
package_dir="$npm_config_prefix/lib/node_modules/http-server"
mkdir -p "$package_dir/bin"
mkdir -p "$package_dir/node_modules/other/bin"
cat > "$package_dir/package.json" <<'EOF'
{"name":"http-server","bin":{"http-server":"bin/http-server"}}
EOF
cat > "$package_dir/bin/http-server" <<'EOF'
#!/bin/sh
echo "primary"
EOF
cat > "$package_dir/node_modules/other/bin/http-server" <<'EOF'
#!/bin/sh
echo "nested"
EOF
chmod +x "$package_dir/bin/http-server"
chmod +x "$package_dir/node_modules/other/bin/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-npm-prefix");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server-nested-dep",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir
        .join("lib")
        .join("node_modules")
        .join("http-server")
        .join("bin")
        .join("http-server");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
}

#[cfg(unix)]
#[test]
fn npm_global_pnpm_prepends_pnpm_home_to_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_pnpm = fake_bin_dir.join("pnpm");
    write_executable(
        &fake_pnpm,
        r#"#!/bin/sh
[ -n "$PNPM_HOME" ] || exit 9
case ":$PATH:" in
  *":$PNPM_HOME:"*) ;;
  *) exit 10 ;;
esac
mkdir -p "$PNPM_HOME"
cat > "$PNPM_HOME/http-server" <<'EOF'
#!/bin/sh
echo "14.1.1"
EOF
chmod +x "$PNPM_HOME/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-pnpm-home");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server-pnpm",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
            "--manager",
            "pnpm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir.join("http-server");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source_kind"], "npm_global");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn npm_global_bun_uses_managed_dir_bin_subdirectory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_bun = fake_bin_dir.join("bun");
    write_executable(
        &fake_bun,
        r#"#!/bin/sh
[ -n "$BUN_INSTALL_GLOBAL_DIR" ] || exit 9
[ -n "$BUN_INSTALL_BIN" ] || exit 10
case ":$PATH:" in
  *":$BUN_INSTALL_BIN:"*) ;;
  *) exit 11 ;;
esac
mkdir -p "$BUN_INSTALL_GLOBAL_DIR"
mkdir -p "$BUN_INSTALL_BIN"
cat > "$BUN_INSTALL_BIN/http-server" <<'EOF'
#!/bin/sh
echo "14.1.1"
EOF
chmod +x "$BUN_INSTALL_BIN/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-bun-root");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server-bun",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
            "--manager",
            "bun",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir.join("bin").join("http-server");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn npm_global_bun_falls_back_to_discovered_executable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_bun = fake_bin_dir.join("bun");
    write_executable(
        &fake_bun,
        r#"#!/bin/sh
[ -n "$BUN_INSTALL_GLOBAL_DIR" ] || exit 9
[ -n "$BUN_INSTALL_BIN" ] || exit 10
mkdir -p "$BUN_INSTALL_GLOBAL_DIR/node_modules/.bin"
cat > "$BUN_INSTALL_GLOBAL_DIR/node_modules/.bin/http-server" <<'EOF'
#!/bin/sh
echo "14.1.1"
EOF
chmod +x "$BUN_INSTALL_GLOBAL_DIR/node_modules/.bin/http-server"
"#,
    );

    let managed_dir = temp.path().join("custom-bun-root");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "npm_global",
            "--id",
            "http-server-bun-fallback",
            "--package",
            "http-server@14.1.1",
            "--binary-name",
            "http-server",
            "--manager",
            "bun",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir
        .join("install")
        .join("global")
        .join("node_modules")
        .join(".bin")
        .join("http-server");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn cargo_install_reports_root_bin_destination_for_custom_managed_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_cargo = fake_bin_dir.join("cargo");
    write_executable(
        &fake_cargo,
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
[ -n "$root" ] || exit 9
mkdir -p "$root/bin"
cat > "$root/bin/demo-cargo" <<'EOF'
#!/bin/sh
echo "demo-cargo 0.1.0"
EOF
chmod +x "$root/bin/demo-cargo"
"#,
    );

    let managed_dir = temp.path().join("custom-managed");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "cargo_install",
            "--id",
            "demo-cargo",
            "--package",
            "demo-cargo-crate",
            "--binary-name",
            "demo-cargo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir.join("bin").join("demo-cargo");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source_kind"], "cargo_install");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn go_install_reports_source_kind_on_success() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_go = fake_bin_dir.join("go");
    write_executable(
        &fake_go,
        r#"#!/bin/sh
[ -n "$GOBIN" ] || exit 9
mkdir -p "$GOBIN"
cat > "$GOBIN/demo-go" <<'EOF'
#!/bin/sh
echo "demo-go"
EOF
chmod +x "$GOBIN/demo-go"
"#,
    );

    let managed_dir = temp.path().join("custom-managed");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "go_install",
            "--id",
            "demo-go",
            "--package",
            "example.com/demo@latest",
            "--binary-name",
            "demo-go",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir.join("demo-go");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source_kind"], "go_install");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[cfg(unix)]
#[test]
fn cargo_install_rejects_stale_binary_when_install_creates_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_cargo = fake_bin_dir.join("cargo");
    write_executable(&fake_cargo, "#!/bin/sh\nexit 0\n");

    let managed_dir = temp.path().join("custom-managed");
    let stale_binary = managed_dir.join("bin").join("demo-cargo");
    std::fs::create_dir_all(stale_binary.parent().expect("stale parent"))
        .expect("create stale parent");
    write_executable(
        &stale_binary,
        r#"#!/bin/sh
echo "stale"
"#,
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--strict",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "cargo_install",
            "--id",
            "demo-cargo",
            "--package",
            "demo-cargo-crate",
            "--binary-name",
            "demo-cargo",
        ])
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "failed");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
    assert_eq!(
        std::fs::read_to_string(&stale_binary).expect("read stale"),
        "#!/bin/sh\necho \"stale\"\n"
    );
    assert!(
        !stale_binary
            .with_file_name("demo-cargo.toolchain-installer-backup")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn go_install_rejects_stale_binary_when_install_creates_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_go = fake_bin_dir.join("go");
    write_executable(&fake_go, "#!/bin/sh\nexit 0\n");

    let managed_dir = temp.path().join("custom-managed");
    let stale_binary = managed_dir.join("demo-go");
    std::fs::create_dir_all(&managed_dir).expect("create managed dir");
    write_executable(
        &stale_binary,
        r#"#!/bin/sh
echo "stale"
"#,
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--strict",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "go_install",
            "--id",
            "demo-go",
            "--package",
            "example.com/demo@latest",
            "--binary-name",
            "demo-go",
        ])
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "failed");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
    assert_eq!(
        std::fs::read_to_string(&stale_binary).expect("read stale"),
        "#!/bin/sh\necho \"stale\"\n"
    );
    assert!(
        !stale_binary
            .with_file_name("demo-go.toolchain-installer-backup")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn go_install_invalid_local_path_preserves_existing_binary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_go = fake_bin_dir.join("go");
    write_executable(&fake_go, "#!/bin/sh\nexit 99\n");

    let managed_dir = temp.path().join("custom-managed");
    let stale_binary = managed_dir.join("demo-go");
    std::fs::create_dir_all(&managed_dir).expect("create managed dir");
    write_executable(
        &stale_binary,
        r#"#!/bin/sh
echo "stale"
"#,
    );
    let missing_source = temp.path().join("missing-package");

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--strict",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "go_install",
            "--id",
            "demo-go",
            "--package",
            missing_source.to_str().expect("utf8 path"),
            "--binary-name",
            "demo-go",
        ])
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "failed");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
    assert!(
        json["items"][0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("go_install local path does not exist"))
    );
    assert_eq!(
        std::fs::read_to_string(&stale_binary).expect("read stale"),
        "#!/bin/sh\necho \"stale\"\n"
    );
    assert!(
        !stale_binary
            .with_file_name("demo-go.toolchain-installer-backup")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn rustup_component_reports_source_kind_on_success() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_rustup = fake_bin_dir.join("rustup");
    let fake_rustfmt = fake_bin_dir.join("rustfmt");
    write_executable(
        &fake_rustup,
        r#"#!/bin/sh
[ "$1" = "component" ] || exit 9
[ "$2" = "add" ] || exit 10
[ "$3" = "rustfmt" ] || exit 11
"#,
    );
    write_executable(
        &fake_rustfmt,
        r#"#!/bin/sh
echo "rustfmt"
"#,
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--method",
            "rustup_component",
            "--id",
            "rustfmt",
            "--package",
            "rustfmt",
            "--binary-name",
            "rustfmt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source_kind"], "rustup_component");
    assert_eq!(
        json["items"][0]["destination"],
        fake_rustfmt.display().to_string()
    );
}

#[cfg(unix)]
#[test]
fn plan_file_resolves_local_paths_relative_to_plan_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_bin_dir = temp.path().join("fake-bin");
    let fake_cargo = fake_bin_dir.join("cargo");
    write_executable(
        &fake_cargo,
        r#"#!/bin/sh
root=""
path=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--root" ]; then
    root="$2"
    shift 2
    continue
  fi
  if [ "$1" = "--path" ]; then
    path="$2"
    shift 2
    continue
  fi
  shift
done
[ -d "$path" ] || exit 9
[ -f "$path/Cargo.toml" ] || exit 10
[ -n "$root" ] || exit 11
mkdir -p "$root/bin"
cat > "$root/bin/demo-cli" <<'EOF'
#!/bin/sh
echo "demo-cli 0.1.0"
EOF
chmod +x "$root/bin/demo-cli"
"#,
    );

    let plan_dir = temp.path().join("plan");
    let package_dir = plan_dir.join("demo-crate");
    let other_cwd = temp.path().join("other");
    std::fs::create_dir_all(&package_dir).expect("create package dir");
    std::fs::create_dir_all(&other_cwd).expect("create other cwd");
    std::fs::write(
        package_dir.join("Cargo.toml"),
        r#"[package]
name = "demo-cli"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("write Cargo.toml");
    let plan_path = plan_dir.join("plan.json");
    std::fs::create_dir_all(&plan_dir).expect("create plan dir");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    {
      "id": "demo-cli",
      "method": "cargo_install",
      "package": "./demo-crate",
      "binary_name": "demo-cli"
    }
  ]
}"#,
    )
    .expect("write plan");

    let managed_dir = temp.path().join("managed");
    let mut cmd = bootstrap_cmd();
    let output = cmd
        .current_dir(&other_cwd)
        .env("PATH", path_with_prepend(&fake_bin_dir))
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--plan-file",
        ])
        .arg(&plan_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    let expected = managed_dir.join("bin").join("demo-cli");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        expected.display().to_string()
    );
    assert!(expected.exists());
}

#[test]
fn conflicting_nested_plan_destinations_return_usage_exit_code() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("nested-conflict-plan.json");
    std::fs::write(
        &plan_path,
        r#"{
  "schema_version": 1,
  "items": [
    { "id": "sdk-tree", "method": "archive_tree_release", "url": "https://example.com/sdk.tar.gz", "destination": "sdk" },
    { "id": "sdk-launcher", "method": "release", "url": "https://example.com/sdk-launcher.tar.gz", "destination": "sdk/bin/demo" }
  ]
}"#,
    )
    .expect("write plan");

    let mut cmd = bootstrap_cmd();
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn apt_rejects_non_apt_manager() {
    let mut cmd = bootstrap_cmd();
    cmd.args([
        "--method",
        "apt",
        "--id",
        "demo-apt",
        "--package",
        "demo-package",
        "--manager",
        "dnf",
    ])
    .assert()
    .code(2);
}

#[cfg_attr(
    windows,
    ignore = "windows uses the real uv installer here, so mirror fallback is not deterministic"
)]
#[test]
fn uv_python_method_accepts_tool_version_and_python_mirror() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("managed dir");
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
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "uv_python",
            "--id",
            "python3.13.12",
            "--tool-version",
            "3.13.12",
            "--python-mirror",
            "https://mirror.example/python",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["source"],
        "python-mirror:https://mirror.example/python"
    );
    assert_eq!(json["items"][0]["source_kind"], "python_mirror");
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir.join("python3.13").display().to_string()
    );
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[test]
fn uv_python_method_ignores_inherited_uv_environment() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("managed dir");
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "python" ] && [ "$2" = "install" ]; then
  if [ -n "$UV_PYTHON_INSTALL_MIRROR" ]; then
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
    );

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("UV_PYTHON_INSTALL_MIRROR", "https://host.example/python")
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--method",
            "uv_python",
            "--id",
            "python3.13.12",
            "--tool-version",
            "3.13.12",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir.join("python3.13").display().to_string()
    );
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[test]
fn uv_tool_method_accepts_binary_name_and_explicit_package_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("managed dir");
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  echo "$UV_DEFAULT_INDEX" > "$UV_TOOL_BIN_DIR/index.log"
  mkdir -p "$UV_TOOL_BIN_DIR"
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
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS", "1")
        .env_remove("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES")
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--package-index",
            &index,
            "--method",
            "uv_tool",
            "--id",
            "ruff-lsp-installer",
            "--package",
            "ruff-lsp",
            "--binary-name",
            "ruff-lsp",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(json["items"][0]["source"], format!("package-index:{index}"));
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir.join("ruff-lsp").display().to_string()
    );
    assert_eq!(
        std::fs::read_to_string(managed_dir.join("index.log"))
            .expect("read explicit index log")
            .trim(),
        index
    );
    assert!(managed_dir.join("ruff-lsp").exists());

    handle.join().expect("mock server thread join");
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[test]
fn uv_tool_method_ignores_inherited_uv_environment() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("managed dir");
    write_executable(
        &managed_dir.join("uv"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "uv 0.11.0"
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "install" ]; then
  if [ -n "$UV_EXTRA_INDEX_URL" ]; then
    echo "inherited extra index leaked" >&2
    exit 19
  fi
  mkdir -p "$UV_TOOL_BIN_DIR"
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
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS", "1")
        .env("UV_EXTRA_INDEX_URL", "https://host.example/simple")
        .env_remove("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES")
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--package-index",
            &index,
            "--method",
            "uv_tool",
            "--id",
            "ruff-lsp-installer",
            "--package",
            "ruff-lsp",
            "--binary-name",
            "ruff-lsp",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["status"], "installed");
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir.join("ruff-lsp").display().to_string()
    );

    handle.join().expect("mock server thread join");
}

#[cfg_attr(windows, ignore = "mock uv shim is unix-specific")]
#[test]
fn uv_tool_method_fails_when_managed_binary_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("managed dir");
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
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("server addr");
    let index = format!("http://{addr}/simple");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert("/simple".to_string(), b"ok".to_vec());
    let handle = spawn_mock_http_server(listener, routes, 1);

    let mut cmd = bootstrap_cmd();
    let output = cmd
        .env("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS", "1")
        .env_remove("TOOLCHAIN_INSTALLER_PACKAGE_INDEXES")
        .args([
            "--json",
            "--managed-dir",
            managed_dir.to_str().expect("utf8 path"),
            "--package-index",
            &index,
            "--method",
            "uv_tool",
            "--id",
            "ruff-lsp-installer",
            "--package",
            "ruff-lsp",
            "--binary-name",
            "ruff-lsp",
        ])
        .assert()
        .code(4)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["error_code"], "install_failed");
    assert_eq!(
        json["items"][0]["destination"],
        managed_dir.join("ruff-lsp").display().to_string()
    );
    assert!(
        json["items"][0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("expected managed binary")
    );

    handle.join().expect("mock server thread join");
}

fn write_executable(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).expect("stat script").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod script");
    }
}

fn make_tar_gz_archive(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, body, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, *path, &mut Cursor::new(*body))
            .expect("append tar entry");
    }
    let encoder = builder.into_inner().expect("finalize tar builder");
    encoder.finish().expect("finalize gzip stream")
}

fn make_tar_gz_archive_with_symlinks(
    file_entries: &[(&str, &[u8], u32)],
    symlink_entries: &[(&str, &str, u32)],
) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, body, mode) in file_entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, *path, &mut Cursor::new(*body))
            .expect("append tar file entry");
    }
    for (path, target, mode) in symlink_entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(*mode);
        header.set_link_name(*target).expect("set symlink target");
        header.set_cksum();
        builder
            .append_data(&mut header, *path, std::io::empty())
            .expect("append tar symlink entry");
    }
    let encoder = builder.into_inner().expect("finalize tar builder");
    encoder.finish().expect("finalize gzip stream")
}

fn make_zip_archive(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut writer = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut writer);
        for (path, body, mode) in entries {
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(*mode);
            archive.start_file(*path, options).expect("start zip entry");
            archive.write_all(body).expect("write zip entry");
        }
        archive.finish().expect("finish zip archive");
    }
    writer.into_inner()
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
