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
        ArtifactDownloadCandidateKind::Mirror => BootstrapSourceKind::Mirror,
        ArtifactDownloadCandidateKind::Canonical => BootstrapSourceKind::Canonical,
    }
}

pub(crate) fn redact_source_url(raw: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return raw.to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
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

#[cfg(test)]
mod tests {
    use omne_artifact_install_primitives::ArtifactDownloadCandidateKind;

    use super::{
        build_download_candidates, redact_source_url, result_source_kind_for_download_candidate,
    };
    use crate::contracts::BootstrapSourceKind;

    #[test]
    fn build_download_candidates_sets_gateway_canonical_and_mirror_kinds() {
        let candidates = build_download_candidates(
            "https://example.com/demo.tar.gz",
            &["https://mirror.example/".to_string()],
            Some("https://gateway.example/demo.tar.gz"),
        );

        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].kind, ArtifactDownloadCandidateKind::Gateway);
        assert_eq!(candidates[1].kind, ArtifactDownloadCandidateKind::Canonical);
        assert_eq!(candidates[2].kind, ArtifactDownloadCandidateKind::Mirror);
    }

    #[test]
    fn result_source_kind_maps_known_source_kinds() {
        assert_eq!(
            result_source_kind_for_download_candidate(ArtifactDownloadCandidateKind::Gateway),
            BootstrapSourceKind::Gateway
        );
        assert_eq!(
            result_source_kind_for_download_candidate(ArtifactDownloadCandidateKind::Canonical),
            BootstrapSourceKind::Canonical
        );
        assert_eq!(
            result_source_kind_for_download_candidate(ArtifactDownloadCandidateKind::Mirror),
            BootstrapSourceKind::Mirror
        );
    }

    #[test]
    fn redact_source_url_strips_credentials_query_and_fragment() {
        assert_eq!(
            redact_source_url(
                "https://user:secret@example.com/download/demo.tar.gz?token=abc#frag"
            ),
            "https://example.com/download/demo.tar.gz"
        );
    }

    #[test]
    fn redact_source_url_keeps_unparseable_input_verbatim() {
        assert_eq!(redact_source_url("not a url"), "not a url");
    }
}
