use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

#[test]
fn bootstrap_with_unknown_tool_returns_unsupported_status() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["--json", "--tool", "custom-tool"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(json["schema_version"], 1);
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
        .failure();
}

#[test]
fn method_without_id_returns_failure() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--method", "pip"]).assert().failure();
}

#[test]
fn missing_plan_file_returns_failure() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--plan-file", "/tmp/not-exist-plan-file.json"])
        .assert()
        .failure();
}

#[test]
fn invalid_plan_file_json_returns_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan_path = temp.path().join("broken-plan.json");
    std::fs::write(&plan_path, "{ invalid").expect("write plan");

    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    cmd.args(["--plan-file"]).arg(&plan_path).assert().failure();
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
        "pip-missing-package",
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
fn default_bootstrap_json_shape_contains_target_and_items() {
    let mut cmd = cargo_bin_cmd!("toolchain-installer");
    let output = cmd
        .args(["--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("valid json");
    assert!(json["target_triple"].as_str().unwrap_or_default().len() > 3);
    let items = json["items"].as_array().expect("items array");
    assert!(!items.is_empty());
}
