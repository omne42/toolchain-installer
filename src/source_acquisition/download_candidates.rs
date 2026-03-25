#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadCandidateKind {
    Gateway,
    Canonical,
    Mirror,
}

impl DownloadCandidateKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Canonical => "canonical",
            Self::Mirror => "mirror",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DownloadCandidate {
    pub(crate) url: String,
    pub(crate) kind: DownloadCandidateKind,
}

pub(crate) fn build_download_candidates(
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
) -> Vec<DownloadCandidate> {
    let mut out = Vec::new();
    if let Some(gateway) = gateway_candidate {
        let trimmed = gateway.trim();
        if !trimmed.is_empty() {
            out.push(DownloadCandidate {
                url: trimmed.to_string(),
                kind: DownloadCandidateKind::Gateway,
            });
        }
    }
    out.push(DownloadCandidate {
        url: canonical_url.to_string(),
        kind: DownloadCandidateKind::Canonical,
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
            out.push(DownloadCandidate {
                url: candidate,
                kind: DownloadCandidateKind::Mirror,
            });
        }
    }
    out
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
