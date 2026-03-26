mod application;
mod artifact;
mod builtin_tools;
mod contracts;
mod download_sources;
mod error;
mod external_gateway;
mod install_plan;
mod installer_runtime_config;
mod managed_toolchain;
mod plan_items;

pub use application::bootstrap_use_case::bootstrap;
pub use application::install_plan_use_case::apply_install_plan;
pub use contracts::{
    BootstrapArchiveFormat, BootstrapArchiveMatch, BootstrapCommand, BootstrapItem,
    BootstrapResult, BootstrapSourceKind, BootstrapStatus, ExecutionRequest, InstallPlan,
    InstallPlanItem, OUTPUT_SCHEMA_VERSION, PLAN_SCHEMA_VERSION, has_failure,
};
pub use error::{ExitCode, InstallerError, InstallerResult};
pub use install_plan::install_plan_validation::validate_install_plan;

#[cfg(test)]
mod lib_tests;
