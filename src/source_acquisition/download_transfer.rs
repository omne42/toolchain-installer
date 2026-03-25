use std::io::Write;

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
    read_response_body_into_writer(response, &candidate.url, writer, options).await
}

async fn read_response_body_into_writer<W>(
    mut response: reqwest::Response,
    url: &str,
    writer: &mut W,
    options: DownloadOptions,
) -> anyhow::Result<()>
where
    W: Write + ?Sized,
{
    if let (Some(limit), Some(content_length)) = (options.max_bytes, response.content_length()) {
        ensure_download_size_within_limit(content_length, limit, url)?;
    }

    let mut downloaded_bytes = 0_u64;
    while let Some(chunk) = response.chunk().await? {
        downloaded_bytes = downloaded_bytes
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("download size overflow"))?;
        if let Some(limit) = options.max_bytes {
            ensure_download_size_within_limit(downloaded_bytes, limit, url)?;
        }
        writer.write_all(&chunk)?;
    }
    Ok(())
}

fn ensure_download_size_within_limit(size: u64, limit: u64, url: &str) -> anyhow::Result<()> {
    if size > limit {
        return Err(anyhow::anyhow!(
            "response body size {size} exceeds configured max download size {limit} for {url}"
        ));
    }
    Ok(())
}
