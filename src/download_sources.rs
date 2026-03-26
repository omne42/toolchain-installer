use omne_artifact_install_primitives::{ArtifactDownloadCandidate, ArtifactDownloadCandidateKind};

use crate::contracts::BootstrapSourceKind;

pub(crate) fn build_download_candidates(
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
) -> Vec<ArtifactDownloadCandidate> {
    let mut out = Vec::new();
    if let Some(gateway) = gateway_candidate {
        let trimmed = gateway.trim();
        if !trimmed.is_empty() {
            out.push(ArtifactDownloadCandidate {
                url: trimmed.to_string(),
                kind: ArtifactDownloadCandidateKind::Gateway,
            });
        }
    }
    out.push(ArtifactDownloadCandidate {
        url: canonical_url.to_string(),
        kind: ArtifactDownloadCandidateKind::Canonical,
    });
    for raw_prefix in mirror_prefixes {
        let prefix = raw_prefix.trim();
        if prefix.is_empty() {
            continue;
        }
        let candidate = if prefix.contains("{url}") {
            prefix.replace("{url}", canonical_url)
        } else {
            format!("{prefix}{canonical_url}")
        };
        if !out.iter().any(|value| value.url == candidate) {
            out.push(ArtifactDownloadCandidate {
                url: candidate,
                kind: ArtifactDownloadCandidateKind::Mirror,
            });
        }
    }
    out
}

pub(crate) fn result_source_kind_for_download_candidate(
    kind: ArtifactDownloadCandidateKind,
) -> BootstrapSourceKind {
    match kind {
        ArtifactDownloadCandidateKind::Gateway => BootstrapSourceKind::Gateway,
        ArtifactDownloadCandidateKind::Canonical => BootstrapSourceKind::Canonical,
        ArtifactDownloadCandidateKind::Mirror => BootstrapSourceKind::Mirror,
    }
}

#[cfg(test)]
pub(crate) fn make_download_candidates(
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
) -> Vec<String> {
    build_download_candidates(canonical_url, mirror_prefixes, gateway_candidate)
        .into_iter()
        .map(|candidate| candidate.url)
        .collect()
}
