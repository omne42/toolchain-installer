pub(crate) const SUPPORTED_PLAN_METHODS: &[&str] = &[
    "release",
    "archive_tree_release",
    "system_package",
    "apt",
    "pip",
    "npm_global",
    "workspace_package",
    "cargo_install",
    "rustup_component",
    "go_install",
    "uv",
    "uv_python",
    "uv_tool",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedToolchainMethod {
    Uv,
    UvPython,
    UvTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanMethod {
    Release,
    ArchiveTreeRelease,
    SystemPackage,
    Apt,
    Pip,
    NpmGlobal,
    WorkspacePackage,
    CargoInstall,
    RustupComponent,
    GoInstall,
    ManagedToolchain(ManagedToolchainMethod),
    Unknown,
}

impl PlanMethod {
    pub(crate) fn from_normalized(normalized: &str) -> Self {
        match normalized {
            "release" => Self::Release,
            "archive_tree_release" => Self::ArchiveTreeRelease,
            "system_package" => Self::SystemPackage,
            "apt" => Self::Apt,
            "pip" => Self::Pip,
            "npm_global" => Self::NpmGlobal,
            "workspace_package" => Self::WorkspacePackage,
            "cargo_install" => Self::CargoInstall,
            "rustup_component" => Self::RustupComponent,
            "go_install" => Self::GoInstall,
            "uv" => Self::ManagedToolchain(ManagedToolchainMethod::Uv),
            "uv_python" => Self::ManagedToolchain(ManagedToolchainMethod::UvPython),
            "uv_tool" => Self::ManagedToolchain(ManagedToolchainMethod::UvTool),
            _ => Self::Unknown,
        }
    }

    pub(crate) fn is_host_bound(self) -> bool {
        matches!(
            self,
            Self::SystemPackage
                | Self::Apt
                | Self::Pip
                | Self::NpmGlobal
                | Self::WorkspacePackage
                | Self::CargoInstall
                | Self::RustupComponent
                | Self::GoInstall
                | Self::ManagedToolchain(_)
        )
    }
}

pub(crate) fn normalize_plan_method(raw_method: &str) -> Option<String> {
    let normalized = raw_method.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_plan_method_rejects_empty_values() {
        assert!(normalize_plan_method("").is_none());
        assert!(normalize_plan_method("   ").is_none());
    }

    #[test]
    fn classify_managed_toolchain_methods() {
        assert_eq!(
            PlanMethod::from_normalized("uv_python"),
            PlanMethod::ManagedToolchain(ManagedToolchainMethod::UvPython)
        );
        assert_eq!(
            PlanMethod::from_normalized("uv_tool"),
            PlanMethod::ManagedToolchain(ManagedToolchainMethod::UvTool)
        );
    }

    #[test]
    fn host_bound_methods_include_managed_toolchain() {
        assert!(PlanMethod::ManagedToolchain(ManagedToolchainMethod::Uv).is_host_bound());
        assert!(!PlanMethod::Release.is_host_bound());
    }
}
