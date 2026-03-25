#[cfg(test)]
use std::io::Cursor;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(test)]
use omne_archive_primitives::extract_binary_from_archive;
use omne_archive_primitives::{
    ArchiveBinaryMatch, BinaryArchiveRequest, extract_binary_from_archive_reader_to_writer,
    is_binary_archive_asset_name,
};
#[cfg(test)]
use omne_fs_primitives::write_file_atomically_from_reader;
use omne_fs_primitives::{
    AtomicWriteOptions, stage_file_atomically, stage_file_atomically_with_name,
};
use omne_integrity_primitives::{Sha256Digest, verify_sha256_reader};

use crate::error::{OperationError, OperationResult};
use crate::source_acquisition::{
    DownloadCandidate, DownloadOptions, build_download_candidates,
    download_candidate_to_writer_with_options,
};

#[derive(Debug)]
pub(crate) struct InstalledArchiveDownload {
    pub(crate) source: DownloadCandidate,
    pub(crate) archive_match: ArchiveBinaryMatch,
}

#[cfg(test)]
pub(crate) fn install_binary_from_archive(
    asset_name: &str,
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &Path,
    archive_binary_hint: Option<&str>,
) -> OperationResult<()> {
    let extracted = extract_binary_from_archive(
        asset_name,
        content,
        &BinaryArchiveRequest {
            binary_name,
            tool_name: tool,
            archive_binary_hint,
        },
    )
    .map_err(|err| OperationError::install(err.to_string()))?;
    let mut reader = Cursor::new(extracted.bytes);
    write_binary_from_reader(&mut reader, destination)
}

pub(crate) fn install_binary_from_archive_reader<R>(
    asset_name: &str,
    reader: R,
    binary_name: &str,
    tool: &str,
    destination: &Path,
    archive_binary_hint: Option<&str>,
) -> OperationResult<ArchiveBinaryMatch>
where
    R: Read + Seek,
{
    let mut staged = stage_file_atomically(destination, &binary_write_options())
        .map_err(|err| OperationError::install(err.to_string()))?;
    let matched = extract_binary_from_archive_reader_to_writer(
        asset_name,
        reader,
        &BinaryArchiveRequest {
            binary_name,
            tool_name: tool,
            archive_binary_hint,
        },
        staged.file_mut(),
    )
    .map_err(|err| OperationError::install(err.to_string()))?;
    staged
        .commit()
        .map_err(|err| OperationError::install(err.to_string()))?;
    Ok(matched)
}

pub(crate) fn is_supported_archive_asset_name(asset_name: &str) -> bool {
    is_binary_archive_asset_name(asset_name)
}

#[cfg(test)]
pub(crate) fn write_binary_from_reader(
    reader: &mut dyn Read,
    destination: &Path,
) -> OperationResult<()> {
    let options = binary_write_options();
    write_file_atomically_from_reader(reader, destination, &options)
        .map_err(|err| OperationError::install(err.to_string()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_and_install_binary_from_archive(
    client: &reqwest::Client,
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
    destination: &Path,
    asset_name: &str,
    binary_name: &str,
    tool: &str,
    archive_binary_hint: Option<&str>,
    expected_sha: Option<&Sha256Digest>,
    max_download_bytes: Option<u64>,
) -> OperationResult<InstalledArchiveDownload> {
    let candidates = build_download_candidates(canonical_url, mirror_prefixes, gateway_candidate);
    let mut errors = Vec::new();
    for candidate in candidates {
        let mut staged = stage_file_atomically_with_name(
            destination,
            &archive_download_stage_options(),
            Some(asset_name),
        )
        .map_err(|err| OperationError::install(err.to_string()))?;
        let download_result = download_candidate_to_writer_with_options(
            client,
            &candidate,
            staged.file_mut(),
            DownloadOptions {
                max_bytes: max_download_bytes,
            },
        )
        .await;
        if let Err(err) = download_result {
            errors.push(format!(
                "{}:{} -> {err}",
                candidate.kind.label(),
                candidate.url
            ));
            continue;
        }

        if let Some(expected_sha) = expected_sha {
            staged
                .file_mut()
                .seek(SeekFrom::Start(0))
                .map_err(|err| OperationError::install(err.to_string()))?;
            verify_sha256_reader(staged.file_mut(), expected_sha)
                .map_err(|err| OperationError::download(err.to_string()))?;
        }
        staged
            .file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(|err| OperationError::install(err.to_string()))?;
        let matched = install_binary_from_archive_reader(
            asset_name,
            staged.file_mut(),
            binary_name,
            tool,
            destination,
            archive_binary_hint,
        )?;
        return Ok(InstalledArchiveDownload {
            source: candidate,
            archive_match: matched,
        });
    }
    Err(OperationError::download(format!(
        "all download candidates failed for {canonical_url}: {}",
        errors.join(" | ")
    )))
}

fn archive_download_stage_options() -> AtomicWriteOptions {
    AtomicWriteOptions {
        create_parent_directories: true,
        ..AtomicWriteOptions::default()
    }
}

fn binary_write_options() -> AtomicWriteOptions {
    AtomicWriteOptions {
        overwrite_existing: true,
        create_parent_directories: true,
        require_non_empty: true,
        require_executable_on_unix: true,
        unix_mode: Some(0o755),
    }
}
