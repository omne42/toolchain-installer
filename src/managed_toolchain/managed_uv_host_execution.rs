use std::ffi::{OsStr, OsString};
use std::process::Output;
use std::time::Duration;

use omne_process_primitives::{
    HostCommandError, HostCommandRunOptions, HostCommandSudoMode, HostRecipeError,
    HostRecipeRequest, run_host_recipe_with_options,
};

use crate::download_sources::redact_source_url;

const MANAGED_UV_RECIPE_OUTPUT_LIMIT: usize = 64 * 1024;

pub(crate) fn run_managed_uv_recipe(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<Output, String> {
    run_managed_uv_recipe_with_timeout(program, args, env, timeout)
}

fn run_managed_uv_recipe_with_timeout(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<Output, String> {
    run_managed_uv_recipe_with_limits(program, args, env, timeout, MANAGED_UV_RECIPE_OUTPUT_LIMIT)
}

fn run_managed_uv_recipe_with_limits(
    program: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
    timeout: Duration,
    output_limit: usize,
) -> Result<Output, String> {
    let removed_env = inherited_uv_environment_names();
    let request = HostRecipeRequest::new(program, args)
        .with_env(env)
        .with_sudo_mode(HostCommandSudoMode::Never);
    let options = HostCommandRunOptions::new()
        .with_env_remove(&removed_env)
        .with_timeout(timeout);

    match run_host_recipe_with_options(&request, options) {
        Ok(output) => Ok(output.output),
        Err(HostRecipeError::NonZeroExit {
            program, output, ..
        }) => Err(format!(
            "run {} failed: status={} stderr={} stdout={}",
            program.to_string_lossy(),
            output.status,
            render_captured_bytes(&output.stderr, output_limit),
            render_captured_bytes(&output.stdout, output_limit),
        )),
        Err(HostRecipeError::Command(err)) => {
            Err(render_managed_uv_command_error(err, output_limit))
        }
    }
}

fn render_managed_uv_command_error(err: HostCommandError, output_limit: usize) -> String {
    match err {
        HostCommandError::TimedOut {
            program,
            timeout,
            output,
            ..
        } => format!(
            "run {} timed out after {}s: status={} stderr={} stdout={}",
            program.to_string_lossy(),
            timeout.as_secs(),
            output.status,
            render_captured_bytes(&output.stderr, output_limit),
            render_captured_bytes(&output.stdout, output_limit),
        ),
        other => other.to_string(),
    }
}

fn render_captured_bytes(bytes: &[u8], output_limit: usize) -> String {
    let retained = bytes.len().min(output_limit);
    let mut rendered = redact_url_like_text(&String::from_utf8_lossy(&bytes[..retained]));
    if bytes.len() > output_limit {
        rendered.push_str(&format!(" [truncated after {} bytes]", output_limit));
    }
    rendered
}

fn redact_url_like_text(raw: &str) -> String {
    let mut rendered = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    while let Some(start) = find_http_url_start(raw, cursor) {
        rendered.push_str(&raw[cursor..start]);
        let end = find_http_url_end(raw, start);
        rendered.push_str(&redact_detected_url(&raw[start..end]));
        cursor = end;
    }
    rendered.push_str(&raw[cursor..]);
    rendered
}

fn find_http_url_start(raw: &str, cursor: usize) -> Option<usize> {
    let http = raw[cursor..].find("http://").map(|offset| cursor + offset);
    let https = raw[cursor..].find("https://").map(|offset| cursor + offset);
    match (http, https) {
        (Some(http), Some(https)) => Some(http.min(https)),
        (Some(http), None) => Some(http),
        (None, Some(https)) => Some(https),
        (None, None) => None,
    }
}

fn find_http_url_end(raw: &str, start: usize) -> usize {
    raw[start..]
        .find(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        })
        .map(|offset| start + offset)
        .unwrap_or(raw.len())
}

fn redact_detected_url(raw: &str) -> String {
    let redacted = redact_source_url(raw);
    if redacted != raw {
        return redacted;
    }

    let Some(rest) = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
    else {
        return raw.to_string();
    };
    let scheme = if raw.starts_with("https://") {
        "https://"
    } else {
        "http://"
    };
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let rest = rest
        .split_once('@')
        .map(|(_, tail)| tail)
        .unwrap_or(rest)
        .trim_end_matches('/');
    format!("{scheme}{rest}")
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use super::{run_managed_uv_recipe_with_limits, run_managed_uv_recipe_with_timeout};

    fn portable_unix_script(body: &str) -> String {
        format!("#!/usr/bin/env bash\n{body}")
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_times_out_hung_process() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"sleep 5
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_timeout(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_millis(50),
        )
        .expect_err("hung process should time out");

        assert!(err.contains("timed out"));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_timeout_reports_captured_stdio() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"echo "stdout-before-timeout"
echo "stderr-before-timeout" >&2
sleep 5
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_timeout(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_millis(50),
        )
        .expect_err("hung process should time out");

        assert!(err.contains("stdout-before-timeout"));
        assert!(err.contains("stderr-before-timeout"));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_limits_captured_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"printf '0123456789abcdefghijklmnopqrstuvwxyz'
exit 42
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_limits(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_secs(1),
            16,
        )
        .expect_err("large output should be truncated");

        assert!(err.contains("0123456789abcdef"));
        assert!(err.contains("[truncated after 16 bytes]"));
    }

    #[cfg_attr(windows, ignore = "probe script is unix-specific")]
    #[test]
    fn run_managed_uv_recipe_redacts_sensitive_urls_in_error_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script_path = tmp.path().join("uv");
        write_unix_script(
            &script_path,
            &portable_unix_script(
                r#"echo "stderr=https://user:secret@example.com/simple?token=abc#frag" >&2
echo "stdout=https://user:secret@example.com/simple?token=abc#frag"
exit 42
"#,
            ),
        );

        let err = run_managed_uv_recipe_with_timeout(
            script_path.as_os_str(),
            &[],
            &[],
            Duration::from_secs(1),
        )
        .expect_err("non-zero exit should fail");

        assert!(err.contains("https://example.com/simple"));
        assert!(!err.contains("secret"));
        assert!(!err.contains("token=abc"));
        assert!(!err.contains("#frag"));
    }

    fn write_unix_script(path: &Path, body: &str) {
        fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path).expect("stat script").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod script");
        }
    }
}
