use std::path::PathBuf;

use omne_integrity_primitives::Sha256Digest;
use omne_system_package_primitives::SystemPackageManager;
use reqwest::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodePackageManager {
    Npm,
    Pnpm,
    Bun,
}

impl NodePackageManager {
    pub(crate) fn command_name(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Bun => "bun",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemPackageMode {
    Auto,
    Explicit(SystemPackageManager),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GoInstallSource {
    LocalPath(PathBuf),
    PackageSpec(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CargoInstallSource {
    LocalPath(PathBuf),
    RegistryPackage {
        package: String,
        version: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ReleasePlanItem {
    pub(crate) id: String,
    pub(crate) url: Url,
    pub(crate) sha256: Option<Sha256Digest>,
    pub(crate) archive_binary: Option<String>,
    pub(crate) binary_name: Option<String>,
    pub(crate) destination: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchiveTreeReleasePlanItem {
    pub(crate) id: String,
    pub(crate) url: Url,
    pub(crate) sha256: Option<Sha256Digest>,
    pub(crate) destination: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct SystemPackagePlanItem {
    pub(crate) id: String,
    pub(crate) package: String,
    pub(crate) mode: SystemPackageMode,
}

#[derive(Debug, Clone)]
pub(crate) struct PipPlanItem {
    pub(crate) id: String,
    pub(crate) package: String,
    pub(crate) python: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NpmGlobalPlanItem {
    pub(crate) id: String,
    pub(crate) package_spec: String,
    pub(crate) manager: NodePackageManager,
    pub(crate) binary_name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspacePackagePlanItem {
    pub(crate) id: String,
    pub(crate) package_spec: String,
    pub(crate) manager: NodePackageManager,
    pub(crate) destination: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct CargoInstallPlanItem {
    pub(crate) id: String,
    pub(crate) source: CargoInstallSource,
    pub(crate) binary_name: String,
    pub(crate) binary_name_explicit: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RustupComponentPlanItem {
    pub(crate) id: String,
    pub(crate) component: String,
    pub(crate) binary_name: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GoInstallPlanItem {
    pub(crate) id: String,
    pub(crate) source: GoInstallSource,
    pub(crate) binary_name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedUvPlanItem {
    pub(crate) id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct UvPythonPlanItem {
    pub(crate) id: String,
    pub(crate) version: String,
}

#[derive(Debug, Clone)]
pub(crate) struct UvToolPlanItem {
    pub(crate) id: String,
    pub(crate) package: String,
    pub(crate) python: Option<String>,
    pub(crate) binary_name: String,
    pub(crate) binary_name_explicit: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvedPlanItem {
    Release(ReleasePlanItem),
    ArchiveTreeRelease(ArchiveTreeReleasePlanItem),
    SystemPackage(SystemPackagePlanItem),
    Pip(PipPlanItem),
    NpmGlobal(NpmGlobalPlanItem),
    WorkspacePackage(WorkspacePackagePlanItem),
    CargoInstall(CargoInstallPlanItem),
    RustupComponent(RustupComponentPlanItem),
    GoInstall(GoInstallPlanItem),
    Uv(ManagedUvPlanItem),
    UvPython(UvPythonPlanItem),
    UvTool(UvToolPlanItem),
}

impl ResolvedPlanItem {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Release(item) => &item.id,
            Self::ArchiveTreeRelease(item) => &item.id,
            Self::SystemPackage(item) => &item.id,
            Self::Pip(item) => &item.id,
            Self::NpmGlobal(item) => &item.id,
            Self::WorkspacePackage(item) => &item.id,
            Self::CargoInstall(item) => &item.id,
            Self::RustupComponent(item) => &item.id,
            Self::GoInstall(item) => &item.id,
            Self::Uv(item) => &item.id,
            Self::UvPython(item) => &item.id,
            Self::UvTool(item) => &item.id,
        }
    }
}
