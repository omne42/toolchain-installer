use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};

use error_kit::{CliError, CliExitCode, ErrorCategory, ErrorCode, ErrorRecord};
use omne_artifact_install_primitives::{ArtifactInstallError, ArtifactInstallErrorKind};
use omne_process_primitives::HostRecipeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Usage = 2,
    Download = 3,
    Install = 4,
    StrictFailure = 5,
}

impl ExitCode {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

impl CliExitCode for ExitCode {
    fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug)]
pub struct InstallerError(Box<CliError<ExitCode>>);

impl InstallerError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Usage, message)
    }

    pub fn download(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Download, message)
    }

    pub fn install(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Install, message)
    }

    fn new(exit_code: ExitCode, message: impl Into<String>) -> Self {
        Self(Box::new(build_error(exit_code, message)))
    }

    pub fn exit_code(&self) -> ExitCode {
        self.0.exit_code()
    }

    pub fn error_code(&self) -> &str {
        self.0.record().code().as_str()
    }

    pub fn record(&self) -> &ErrorRecord {
        self.0.record()
    }
}

impl Display for InstallerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl StdError for InstallerError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.0.as_ref())
    }
}

pub type InstallerResult<T> = std::result::Result<T, InstallerError>;

#[derive(Debug)]
pub(crate) struct OperationError(Box<CliError<ExitCode>>);

impl OperationError {
    pub(crate) fn download(message: impl Into<String>) -> Self {
        Self(Box::new(build_error(ExitCode::Download, message)))
    }

    pub(crate) fn install(message: impl Into<String>) -> Self {
        Self(Box::new(build_error(ExitCode::Install, message)))
    }

    pub(crate) fn from_artifact_install(err: ArtifactInstallError) -> Self {
        match err.kind() {
            ArtifactInstallErrorKind::Download => Self::download(err.to_string()),
            ArtifactInstallErrorKind::Install => Self::install(err.to_string()),
        }
    }

    pub(crate) fn from_host_recipe(err: HostRecipeError) -> Self {
        Self::install(err.to_string())
    }

    pub(crate) fn exit_code(&self) -> ExitCode {
        self.0.exit_code()
    }

    pub(crate) fn error_code(&self) -> &str {
        self.0.record().code().as_str()
    }

    pub(crate) fn detail(&self) -> String {
        self.0.record().display_text().to_string()
    }

    pub(crate) fn into_failure_parts(self) -> (String, String, ExitCode) {
        let detail = self.0.record().display_text().to_string();
        let error_code = self.0.record().code().as_str().to_string();
        let exit_code = self.0.exit_code();
        (detail, error_code, exit_code)
    }
}

pub(crate) type OperationResult<T> = std::result::Result<T, OperationError>;

impl Display for OperationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl StdError for OperationError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.0.as_ref())
    }
}

fn build_error(exit_code: ExitCode, message: impl Into<String>) -> CliError<ExitCode> {
    let message = message.into();
    let record = ErrorRecord::new_freeform(error_code(exit_code), message);
    let record = match exit_code {
        ExitCode::Usage => record.with_category(ErrorCategory::InvalidInput),
        ExitCode::Download => record.with_category(ErrorCategory::ExternalDependency),
        ExitCode::Install | ExitCode::StrictFailure => record,
    };
    record.with_exit_code(exit_code)
}

fn error_code(code: ExitCode) -> ErrorCode {
    ErrorCode::try_new(error_code_label(code)).expect("installer error codes should validate")
}

fn error_code_label(code: ExitCode) -> &'static str {
    match code {
        ExitCode::Usage => "usage_error",
        ExitCode::Download => "download_failed",
        ExitCode::Install => "install_failed",
        ExitCode::StrictFailure => "strict_failure",
    }
}
