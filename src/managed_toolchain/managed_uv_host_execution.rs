use std::ffi::{OsStr, OsString};
use std::process::{Command, Output, Stdio};

pub(crate) fn run_managed_uv_recipe(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
) -> Result<Output, String> {
    let mut command = Command::new(program);
    command.args(args);
    for name in inherited_uv_environment_names() {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = command.output().map_err(|err| {
        format!(
            "run {} failed: {err}",
            command.get_program().to_string_lossy()
        )
    })?;
    if output.status.success() {
        return Ok(output);
    }

    Err(format!(
        "run {} failed: status={} stderr={} stdout={}",
        command.get_program().to_string_lossy(),
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    ))
}

fn inherited_uv_environment_names() -> Vec<OsString> {
    std::env::vars_os()
        .filter_map(|(name, _)| {
            name.to_str()
                .is_some_and(|value| value.to_ascii_uppercase().starts_with("UV_"))
                .then_some(name)
        })
        .collect()
}
