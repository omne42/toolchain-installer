mod bootstrap;
mod contracts;
mod error;
mod installation;
mod installer_runtime_config;
mod managed_toolchain;
mod plan;
mod platform;
mod source_acquisition;
mod uv;

pub use bootstrap::builtin_tools::bootstrap;
pub use contracts::{
    BootstrapArchiveFormat, BootstrapArchiveMatch, BootstrapItem, BootstrapRequest,
    BootstrapResult, BootstrapSourceKind, BootstrapStatus, InstallPlan, InstallPlanItem,
    OUTPUT_SCHEMA_VERSION, PLAN_SCHEMA_VERSION, has_failure,
};
pub use error::{ExitCode, InstallerError, InstallerResult};
pub use plan::install_plan_execution::apply_install_plan;
pub use plan::install_plan_validation::validate_install_plan;

#[cfg(test)]
mod lib_tests;
