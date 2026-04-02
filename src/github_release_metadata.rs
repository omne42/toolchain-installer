use github_kit::{GitHubApiRequestOptions, GitHubRelease, fetch_latest_release};
use http_kit::{HttpClientOptions, HttpClientProfile, build_http_client_profile};
#[cfg(test)]
use reqwest::Url;

use crate::error::{OperationError, OperationResult};
use crate::installer_runtime_config::InstallerRuntimeConfig;

pub(crate) async fn fetch_latest_release_metadata(
    _client: &reqwest::Client,
    cfg: &InstallerRuntimeConfig,
    repo: &str,
) -> OperationResult<GitHubRelease> {
    let github_client = build_github_http_client(cfg)?;
    fetch_latest_release(
        &github_client,
        &cfg.github_releases.api_bases,
        repo,
        GitHubApiRequestOptions::new()
            .with_user_agent("toolchain-installer")
            .with_bearer_token(cfg.github_releases.token.as_deref())
            .with_allow_custom_bearer_api_base(true),
    )
    .await
    .map_err(|err| OperationError::download(err.to_string()))
}

pub(crate) fn build_github_http_client(
    cfg: &InstallerRuntimeConfig,
) -> OperationResult<HttpClientProfile> {
    build_http_client_profile(&HttpClientOptions {
        timeout: Some(cfg.download.http_timeout),
        ..HttpClientOptions::default()
    })
    .map_err(|err| OperationError::download(format!("build github http client failed: {err}")))
}

#[cfg(test)]
pub(crate) fn is_github_release_asset_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.host_str() != Some("github.com") {
        return false;
    }
    let Some(segments) = parsed.path_segments() else {
        return false;
    };
    let segments = segments.collect::<Vec<_>>();
    segments.len() >= 6 && segments[2] == "releases" && segments[3] == "download"
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use crate::installer_runtime_config::InstallerRuntimeConfig;
    use crate::installer_runtime_config::{
        DownloadPolicy, DownloadSourcePolicy, GatewayRoutingPolicy, GitHubReleasePolicy,
        PackageIndexPolicy, PythonMirrorPolicy,
    };

    use super::{fetch_latest_release_metadata, is_github_release_asset_url};

    #[tokio::test]
    async fn fetch_latest_release_metadata_uses_shared_github_client_contract_without_bearer_on_http()
     {
        let responses = vec![
            MockHttpResponse {
                expected_path: "/api-fail/repos/cli/cli/releases/latest".to_string(),
                expected_headers: vec![
                    ("accept".to_string(), "application/vnd.github+json".to_string()),
                    ("user-agent".to_string(), "toolchain-installer".to_string()),
                    ("x-github-api-version".to_string(), "2022-11-28".to_string()),
                ],
                status_line: "HTTP/1.1 500 Internal Server Error",
                body: "{\"message\":\"try next\"}".to_string(),
            },
            MockHttpResponse {
                expected_path: "/api-ok/repos/cli/cli/releases/latest".to_string(),
                expected_headers: vec![
                    ("accept".to_string(), "application/vnd.github+json".to_string()),
                    ("user-agent".to_string(), "toolchain-installer".to_string()),
                    ("x-github-api-version".to_string(), "2022-11-28".to_string()),
                ],
                status_line: "HTTP/1.1 200 OK",
                body: r#"{"tag_name":"v2.0.0","assets":[{"name":"asset.tar.gz","browser_download_url":"https://example.invalid/asset.tar.gz","digest":null}]}"#.to_string(),
            },
        ];
        let (base, handle) = spawn_mock_server(responses);
        let cfg = InstallerRuntimeConfig {
            github_releases: GitHubReleasePolicy {
                api_bases: vec![format!("{base}/api-fail"), format!("{base}/api-ok")],
                token: None,
            },
            download_sources: DownloadSourcePolicy {
                mirror_prefixes: Vec::new(),
            },
            package_indexes: PackageIndexPolicy {
                indexes: Vec::new(),
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
        };

        let release = fetch_latest_release_metadata(&reqwest::Client::new(), &cfg, "cli/cli")
            .await
            .expect("release metadata");

        assert_eq!(release.tag_name, "v2.0.0");
        assert_eq!(release.assets.len(), 1);
        handle.join().expect("mock server thread");
    }

    struct MockHttpResponse {
        expected_path: String,
        expected_headers: Vec<(String, String)>,
        status_line: &'static str,
        body: String,
    }

    fn spawn_mock_server(responses: Vec<MockHttpResponse>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept connection");
                let mut request = Vec::new();
                let mut buf = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buf).expect("read request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }

                let request_text = String::from_utf8(request).expect("utf8 request");
                let mut lines = request_text.lines();
                let first = lines.next().expect("request line");
                let path = first
                    .split_whitespace()
                    .nth(1)
                    .expect("request path")
                    .to_string();
                assert_eq!(path, response.expected_path);

                let headers = lines
                    .take_while(|line| !line.is_empty())
                    .filter_map(|line| line.split_once(':'))
                    .map(|(name, value)| {
                        (name.trim().to_ascii_lowercase(), value.trim().to_string())
                    })
                    .collect::<Vec<_>>();
                for (expected_name, expected_value) in response.expected_headers {
                    assert!(
                        headers.iter().any(|(name, value)| {
                            name == &expected_name && value == &expected_value
                        }),
                        "missing header {expected_name}: {expected_value:?} in {headers:?}"
                    );
                }

                let body = response.body;
                write!(
                    stream,
                    "{}\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{}",
                    response.status_line,
                    body.len(),
                    body
                )
                .expect("write response");
            }
        });
        (format!("http://{}", addr), handle)
    }

    #[test]
    fn github_release_asset_url_detection_matches_release_download_shape() {
        assert!(is_github_release_asset_url(
            "https://github.com/cli/cli/releases/download/v2.0.0/gh_2.0.0_linux_amd64.tar.gz"
        ));
        assert!(!is_github_release_asset_url(
            "https://mirror.example/github.com/cli/cli/releases/download/v2.0.0/gh.tar.gz"
        ));
        assert!(!is_github_release_asset_url(
            "https://github.com/cli/cli/archive/refs/tags/v2.0.0.tar.gz"
        ));
    }
}
