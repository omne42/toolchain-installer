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
