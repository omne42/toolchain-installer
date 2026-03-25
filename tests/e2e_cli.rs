use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;

#[test]
fn bootstrap_with_unknown_tool_returns_unsupported_status() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["bootstrap", "--json", "--tool", "custom-tool"])
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

#[test]
fn direct_method_with_unknown_strategy_returns_unsupported_status() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["--json", "--method", "unknown", "--id", "demo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["tool"], "demo");
    assert_eq!(json["items"][0]["status"], "unsupported");
}

#[test]
fn plan_file_path_executes_without_network() {
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

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["--json", "--plan-file"])
        .arg(&plan_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["items"][0]["tool"], "demo");
    assert_eq!(json["items"][0]["status"], "unsupported");
}

#[test]
fn method_and_plan_file_conflict_returns_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("plan.json");
    std::fs::write(&plan_path, r#"{"schema_version":1,"items":[]}"#).expect("write plan");

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--method", "pip", "--id", "demo", "--plan-file"])
        .arg(&plan_path)
        .assert()
        .code(2);
}

#[test]
fn method_without_id_returns_failure() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--method", "pip"]).assert().code(2);
}

#[test]
fn missing_plan_file_returns_failure() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--plan-file", "/tmp/not-exist-plan-file.json"])
        .assert()
        .code(2);
}

#[test]
fn invalid_plan_file_json_returns_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("broken-plan.json");
    std::fs::write(&plan_path, "{ invalid").expect("write plan");

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn strict_mode_fails_when_item_failed() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
fn strict_mode_allows_unsupported_without_failure() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args([
        "--json",
        "--strict",
        "--method",
        "unknown",
        "--id",
        "unsupported-demo",
    ])
    .assert()
    .success();
}

#[test]
fn default_managed_dir_uses_home_omne_data_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fake_home = temp.path().join("home");
    std::fs::create_dir_all(&fake_home).expect("create fake home");

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .args(["--json", "--method", "unknown", "--id", "demo"])
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

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .env("OMNE_DATA_DIR", &omne_data_dir)
        .args(["--json", "--method", "unknown", "--id", "demo"])
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
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["bootstrap", "--json"])
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
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
    let destination = temp.path().join("demo.bin");
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args([
            "--json",
            "--max-download-bytes",
            "4",
            "--method",
            "release",
            "--id",
            "demo-release",
            "--url",
            &format!("http://{addr}/demo.bin"),
            "--destination",
        ])
        .arg(&destination)
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
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["--json", "--method", "unknown", "--id", "host-probe"])
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
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["bootstrap", "--target-triple", &target, "--tool", "git"])
        .assert()
        .code(2);
}

#[test]
fn single_item_install_failure_uses_install_exit_code() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--plan-file"]).arg(&plan_path).assert().code(2);
}

#[test]
fn relative_release_destination_is_resolved_under_managed_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let managed_dir = temp.path().join("managed");
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
            .join("nested/demo-release")
            .display()
            .to_string()
    );
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
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
fn pip_rejects_destination_field() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
fn apt_rejects_non_apt_manager() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
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
