use std::collections::BTreeSet;

pub(crate) fn normalize_requested_tools(input: &[String]) -> Vec<String> {
    if input.is_empty() {
        return vec!["git".to_string(), "gh".to_string()];
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in input {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    if out.is_empty() {
        vec!["git".to_string(), "gh".to_string()]
    } else {
        out
    }
}

pub(crate) fn is_supported_builtin_tool(tool: &str) -> bool {
    matches!(tool, "git" | "gh" | "uv")
}
