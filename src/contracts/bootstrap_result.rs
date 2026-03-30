use omne_archive_primitives::{ArchiveBinaryMatch, BinaryArchiveFormat};
use serde::Serialize;

use crate::error::ExitCode;

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapStatus {
    Present,
    Installed,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapSourceKind {
    Managed,
    Gateway,
    Canonical,
    Mirror,
    SystemPackage,
    Pip,
    CargoInstall,
    GoInstall,
    NpmGlobal,
    WorkspacePackage,
    RustupComponent,
    PythonMirror,
    PackageIndex,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapArchiveFormat {
    TarGz,
    TarXz,
    Zip,
}

impl From<BinaryArchiveFormat> for BootstrapArchiveFormat {
    fn from(value: BinaryArchiveFormat) -> Self {
        match value {
            BinaryArchiveFormat::TarGz => Self::TarGz,
            BinaryArchiveFormat::TarXz => Self::TarXz,
            BinaryArchiveFormat::Zip => Self::Zip,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BootstrapArchiveMatch {
    pub format: BootstrapArchiveFormat,
    pub path: String,
}

impl From<ArchiveBinaryMatch> for BootstrapArchiveMatch {
    fn from(value: ArchiveBinaryMatch) -> Self {
        Self {
            format: value.archive_format.into(),
            path: value.archive_path,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapItem {
    pub tool: String,
    pub status: BootstrapStatus,
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<BootstrapSourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_match: Option<BootstrapArchiveMatch>,
    pub destination: Option<String>,
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing)]
    pub failure_code: Option<ExitCode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapResult {
    pub schema_version: u32,
    pub host_triple: String,
    pub target_triple: String,
    pub managed_dir: String,
    pub items: Vec<BootstrapItem>,
}

pub type InstallExecutionStatus = BootstrapStatus;
pub type InstallExecutionSourceKind = BootstrapSourceKind;
pub type InstallExecutionArchiveFormat = BootstrapArchiveFormat;
pub type InstallExecutionArchiveMatch = BootstrapArchiveMatch;
pub type InstallExecutionItem = BootstrapItem;
pub type InstallExecutionResult = BootstrapResult;

pub fn has_failure(items: &[BootstrapItem]) -> bool {
    items
        .iter()
        .any(|item| item.status == BootstrapStatus::Failed)
}

pub fn has_install_failure(items: &[InstallExecutionItem]) -> bool {
    has_failure(items)
}

pub(crate) fn build_failed_bootstrap_item(
    tool: String,
    destination: Option<String>,
    detail: impl Into<String>,
    error_code: impl Into<String>,
    failure_code: ExitCode,
) -> BootstrapItem {
    BootstrapItem {
        tool,
        status: BootstrapStatus::Failed,
        source: None,
        source_kind: None,
        archive_match: None,
        destination,
        detail: Some(detail.into()),
        error_code: Some(error_code.into()),
        failure_code: Some(failure_code),
    }
}
