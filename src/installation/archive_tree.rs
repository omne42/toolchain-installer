use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive as TarArchive;
use xz2::read::XzDecoder;
use zip::ZipArchive;

use crate::error::{OperationError, OperationResult};

pub(crate) fn is_supported_tree_archive_asset_name(asset_name: &str) -> bool {
    asset_name.ends_with(".tar.gz")
        || asset_name.ends_with(".tar.xz")
        || asset_name.ends_with(".zip")
}

pub(crate) fn extract_archive_tree_from_bytes(
    asset_name: &str,
    archive_bytes: &[u8],
    destination: &Path,
) -> OperationResult<()> {
    reset_destination_dir(destination)?;
    if asset_name.ends_with(".zip") {
        extract_zip_tree(archive_bytes, destination)
    } else if asset_name.ends_with(".tar.gz") {
        extract_tar_tree(GzDecoder::new(Cursor::new(archive_bytes)), destination)
    } else if asset_name.ends_with(".tar.xz") {
        extract_tar_tree(XzDecoder::new(Cursor::new(archive_bytes)), destination)
    } else {
        Err(OperationError::install(format!(
            "unsupported archive_tree_release asset `{asset_name}`"
        )))
    }
}

fn reset_destination_dir(destination: &Path) -> OperationResult<()> {
    if destination.exists() {
        if destination.is_dir() {
            fs::remove_dir_all(destination)
                .map_err(|err| OperationError::install(err.to_string()))?;
        } else {
            fs::remove_file(destination).map_err(|err| OperationError::install(err.to_string()))?;
        }
    }
    fs::create_dir_all(destination).map_err(|err| OperationError::install(err.to_string()))
}

fn extract_zip_tree(archive_bytes: &[u8], destination: &Path) -> OperationResult<()> {
    let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|err| OperationError::install(err.to_string()))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| OperationError::install(err.to_string()))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| {
                OperationError::install(format!("unsafe archive entry path at index {index}"))
            })?
            .to_path_buf();
        let output_path = destination.join(&enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|err| OperationError::install(err.to_string()))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| OperationError::install(err.to_string()))?;
        }
        let mut file =
            File::create(&output_path).map_err(|err| OperationError::install(err.to_string()))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|err| OperationError::install(err.to_string()))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&output_path, fs::Permissions::from_mode(mode))
                .map_err(|err| OperationError::install(err.to_string()))?;
        }
    }
    Ok(())
}

fn extract_tar_tree<R>(reader: R, destination: &Path) -> OperationResult<()>
where
    R: Read,
{
    let mut archive = TarArchive::new(reader);
    let entries = archive
        .entries()
        .map_err(|err| OperationError::install(err.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| OperationError::install(err.to_string()))?;
        let path = entry
            .path()
            .map_err(|err| OperationError::install(err.to_string()))?;
        let sanitized = sanitize_archive_path(&path)?;
        let output_path = destination.join(sanitized);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|err| OperationError::install(err.to_string()))?;
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(OperationError::install(format!(
                "unsupported tar entry type for {}",
                path.display()
            )));
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| OperationError::install(err.to_string()))?;
        }
        entry
            .unpack(&output_path)
            .map_err(|err| OperationError::install(err.to_string()))?;
    }
    Ok(())
}

fn sanitize_archive_path(path: &Path) -> OperationResult<PathBuf> {
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            _ => {
                return Err(OperationError::install(format!(
                    "unsafe tar archive entry path `{}`",
                    path.display()
                )));
            }
        }
    }
    if sanitized.as_os_str().is_empty() {
        return Err(OperationError::install("empty tar archive entry path"));
    }
    Ok(sanitized)
}
