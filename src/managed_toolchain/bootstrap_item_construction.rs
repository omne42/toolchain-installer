use std::path::Path;

use crate::artifact::InstallSource;
use crate::contracts::{BootstrapItem, BootstrapSourceKind, BootstrapStatus};

pub(super) fn build_installed_bootstrap_item_from_install_source(
    tool_id: &str,
    source: InstallSource,
    destination: &Path,
    detail: Option<String>,
) -> BootstrapItem {
    let InstallSource {
        locator,
        source_kind,
        archive_match,
    } = source;
    BootstrapItem {
        tool: tool_id.to_string(),
        status: BootstrapStatus::Installed,
        source: Some(locator),
        source_kind: Some(source_kind),
        archive_match,
        destination: Some(destination.display().to_string()),
        detail,
        error_code: None,
        failure_code: None,
    }
}

pub(super) fn build_installed_bootstrap_item(
    tool_id: &str,
    source_locator: String,
    source_kind: BootstrapSourceKind,
    destination: &Path,
    detail: Option<String>,
) -> BootstrapItem {
    BootstrapItem {
        tool: tool_id.to_string(),
        status: BootstrapStatus::Installed,
        source: Some(source_locator),
        source_kind: Some(source_kind),
        archive_match: None,
        destination: Some(destination.display().to_string()),
        detail,
        error_code: None,
        failure_code: None,
    }
}

pub(super) fn build_managed_uv_usage_detail(
    uv_program: &Path,
    uv_detail: Option<String>,
) -> Option<String> {
    uv_detail.or_else(|| Some(format!("using managed uv `{}`", uv_program.display())))
}
