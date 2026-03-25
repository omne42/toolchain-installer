use std::fs;
use std::path::Path;

const REQUIRED_DOC_FILES: &[&str] = &[
    "AGENTS.md",
    "docs/docs-system-map.md",
    "docs/architecture/system-boundaries.md",
    "docs/architecture/source-layout.md",
    "docs/architecture/worker-gateway-placement.md",
    "docs/contracts/cli-surface.md",
    "docs/contracts/install-plan-contract.md",
    "docs/guides/installation-examples.md",
    "docs/guides/python-toolchain-bootstrap.md",
    "docs/operations/security-boundaries.md",
    "docs/operations/external-gateway-integration.md",
    "docs/operations/quality-and-doc-maintenance.md",
    "docs/plans/delivery-roadmap.md",
    "docs/plans/documentation-tech-debt.md",
    "docs/references/example-plan-files.md",
    "docs/references/source-selection-rules.md",
];

const DEPRECATED_FLAT_DOC_FILES: &[&str] = &[
    "docs/architecture.md",
    "docs/contract.md",
    "docs/examples.md",
    "docs/roadmap.md",
    "docs/security.md",
];

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn required_doc_files_exist() {
    for relative_path in REQUIRED_DOC_FILES {
        assert!(
            repo_root().join(relative_path).exists(),
            "missing required doc file: {relative_path}"
        );
    }
}

#[test]
fn deprecated_flat_doc_files_are_gone() {
    for relative_path in DEPRECATED_FLAT_DOC_FILES {
        assert!(
            !repo_root().join(relative_path).exists(),
            "deprecated flat doc file still exists: {relative_path}"
        );
    }
}

#[test]
fn readme_and_agents_point_to_doc_entrypoints() {
    let readme = fs::read_to_string(repo_root().join("README.md")).expect("read README.md");
    let agents = fs::read_to_string(repo_root().join("AGENTS.md")).expect("read AGENTS.md");

    assert!(readme.contains("docs/docs-system-map.md"));
    assert!(readme.contains("docs/architecture/system-boundaries.md"));
    assert!(readme.contains("docs/contracts/cli-surface.md"));
    assert!(agents.contains("docs/docs-system-map.md"));
    assert!(agents.contains("docs/operations/quality-and-doc-maintenance.md"));
}
