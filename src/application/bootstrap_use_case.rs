use std::path::Path;

use crate::builtin_tools::bootstrap_installation::{
    bootstrap_builtin_tool as execute_bootstrap_builtin_tool, builtin_tool_destination,
};
use crate::builtin_tools::builtin_tool_selection::normalize_requested_tools;
use crate::contracts::{
    BootstrapCommand, BootstrapItem, BootstrapResult, BootstrapStatus, OUTPUT_SCHEMA_VERSION,
};
use crate::error::InstallerResult;
use crate::install_plan::item_destination_resolution::validate_managed_path_boundary;
use crate::installer_runtime_config::InstallerRuntimeConfig;
use omne_host_info_primitives::executable_suffix_for_target;

use super::execution_context::ExecutionContext;

pub async fn bootstrap(command: &BootstrapCommand) -> InstallerResult<BootstrapResult> {
    let ctx = ExecutionContext::for_bootstrap(&command.execution)?;
    let binary_ext = executable_suffix_for_target(&ctx.target_triple);

    let tools = normalize_requested_tools(&command.tools);
    let mut items = Vec::new();
    for tool in tools {
        let item = bootstrap_builtin_tool(
            tool.as_str(),
            &ctx.target_triple,
            binary_ext,
            &ctx.managed_dir,
            &ctx.cfg,
            &ctx.client,
        )
        .await;
        items.push(item);
    }

    Ok(BootstrapResult {
        schema_version: OUTPUT_SCHEMA_VERSION,
        host_triple: ctx.host_triple,
        target_triple: ctx.target_triple,
        managed_dir: ctx.managed_dir.display().to_string(),
        items,
    })
}

async fn bootstrap_builtin_tool(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    managed_dir: &Path,
    cfg: &InstallerRuntimeConfig,
    client: &reqwest::Client,
) -> BootstrapItem {
    let destination = builtin_tool_destination(tool, target_triple, binary_ext, managed_dir);
    if let Err(detail) = validate_managed_path_boundary(&destination, managed_dir, false) {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Failed,
            source: None,
            source_kind: None,
            archive_match: None,
            destination: Some(destination.display().to_string()),
            detail: Some(detail),
            error_code: Some("install_failed".to_string()),
            failure_code: Some(crate::error::ExitCode::Install),
        };
    }
    execute_bootstrap_builtin_tool(
        tool,
        target_triple,
        binary_ext,
        &destination,
        managed_dir,
        cfg,
        client,
    )
    .await
}
