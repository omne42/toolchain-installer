use std::io::Write;

use http_kit::write_response_body_limited;

use super::download_candidates::DownloadCandidate;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DownloadOptions {
    pub(crate) max_bytes: Option<u64>,
}

pub(crate) async fn download_candidate_to_writer_with_options<W>(
    client: &reqwest::Client,
    candidate: &DownloadCandidate,
    writer: &mut W,
    options: DownloadOptions,
) -> anyhow::Result<()>
where
    W: Write + ?Sized,
{
    let response = client.get(&candidate.url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }
    write_response_body_limited(response, writer, options.max_bytes)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
}
