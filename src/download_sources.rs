use omne_artifact_install_primitives::ArtifactDownloadCandidate;

use crate::contracts::BootstrapSourceKind;

const GATEWAY_SOURCE_LABEL: &str = "gateway";
const CANONICAL_SOURCE_LABEL: &str = "canonical";
const MIRROR_SOURCE_LABEL: &str = "mirror";

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
                source_label: GATEWAY_SOURCE_LABEL.to_string(),
            });
        }
    }
    out.push(ArtifactDownloadCandidate {
        url: canonical_url.to_string(),
        source_label: CANONICAL_SOURCE_LABEL.to_string(),
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
                source_label: MIRROR_SOURCE_LABEL.to_string(),
            });
        }
    }
    out
}

pub(crate) fn result_source_kind_for_download_candidate(source_label: &str) -> BootstrapSourceKind {
    match source_label {
        GATEWAY_SOURCE_LABEL => BootstrapSourceKind::Gateway,
        MIRROR_SOURCE_LABEL => BootstrapSourceKind::Mirror,
        CANONICAL_SOURCE_LABEL => BootstrapSourceKind::Canonical,
        _ => BootstrapSourceKind::Canonical,
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
    use super::{
        CANONICAL_SOURCE_LABEL, GATEWAY_SOURCE_LABEL, MIRROR_SOURCE_LABEL,
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
        assert_eq!(candidates[0].source_label, GATEWAY_SOURCE_LABEL);
        assert_eq!(candidates[1].source_label, CANONICAL_SOURCE_LABEL);
        assert_eq!(candidates[2].source_label, MIRROR_SOURCE_LABEL);
    }

    #[test]
    fn result_source_kind_maps_known_source_kinds() {
        assert_eq!(
            result_source_kind_for_download_candidate(GATEWAY_SOURCE_LABEL),
            BootstrapSourceKind::Gateway
        );
        assert_eq!(
            result_source_kind_for_download_candidate(CANONICAL_SOURCE_LABEL),
            BootstrapSourceKind::Canonical
        );
        assert_eq!(
            result_source_kind_for_download_candidate(MIRROR_SOURCE_LABEL),
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
