use crate::contracts::BootstrapSourceKind;

use super::download_candidates::DownloadCandidateKind;

pub(crate) fn result_source_kind_for_download_candidate(
    kind: DownloadCandidateKind,
) -> BootstrapSourceKind {
    match kind {
        DownloadCandidateKind::Gateway => BootstrapSourceKind::Gateway,
        DownloadCandidateKind::Canonical => BootstrapSourceKind::Canonical,
        DownloadCandidateKind::Mirror => BootstrapSourceKind::Mirror,
    }
}
