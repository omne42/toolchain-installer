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
    use super::redact_source_url;

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
