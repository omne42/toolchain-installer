use std::fmt;

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

#[derive(Debug, Clone)]
pub struct InstallerError {
    exit_code: ExitCode,
    message: String,
}

impl InstallerError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: ExitCode::Usage,
            message: message.into(),
        }
    }

    pub fn download(message: impl Into<String>) -> Self {
        Self {
            exit_code: ExitCode::Download,
            message: message.into(),
        }
    }

    pub fn install(message: impl Into<String>) -> Self {
        Self {
            exit_code: ExitCode::Install,
            message: message.into(),
        }
    }

    pub const fn exit_code(&self) -> ExitCode {
        self.exit_code
    }
}

impl fmt::Display for InstallerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InstallerError {}

pub type InstallerResult<T> = std::result::Result<T, InstallerError>;

#[derive(Debug, Clone)]
pub(crate) struct OperationError {
    pub(crate) exit_code: ExitCode,
    pub(crate) message: String,
}

impl OperationError {
    pub(crate) fn download(message: impl Into<String>) -> Self {
        Self {
            exit_code: ExitCode::Download,
            message: message.into(),
        }
    }

    pub(crate) fn install(message: impl Into<String>) -> Self {
        Self {
            exit_code: ExitCode::Install,
            message: message.into(),
        }
    }
}

pub(crate) type OperationResult<T> = std::result::Result<T, OperationError>;

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OperationError {}

pub(crate) fn error_code_label(code: ExitCode) -> &'static str {
    match code {
        ExitCode::Usage => "usage_error",
        ExitCode::Download => "download_failed",
        ExitCode::Install => "install_failed",
        ExitCode::StrictFailure => "strict_failure",
    }
}
