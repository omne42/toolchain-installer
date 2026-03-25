use super::{BootstrapArchiveMatch, BootstrapSourceKind};

#[derive(Debug, Clone)]
pub(crate) struct InstallSource {
    pub(crate) locator: String,
    pub(crate) source_kind: BootstrapSourceKind,
    pub(crate) archive_match: Option<BootstrapArchiveMatch>,
}

impl InstallSource {
    pub(crate) fn new(locator: impl Into<String>, source_kind: BootstrapSourceKind) -> Self {
        Self {
            locator: locator.into(),
            source_kind,
            archive_match: None,
        }
    }

    pub(crate) fn with_archive_match(mut self, archive_match: BootstrapArchiveMatch) -> Self {
        self.archive_match = Some(archive_match);
        self
    }
}
