use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use toolchain_installer::{
    BootstrapCommand, ExecutionRequest, ExitCode, InstallPlan, InstallPlanItem, InstallerError,
    has_failure, validate_install_plan_with_request,
};

#[derive(Args, Debug, Clone, Default)]
struct BootstrapArgs {
    #[arg(long = "tool")]
    tools: Vec<String>,
    #[arg(long)]
    target_triple: Option<String>,
    #[arg(long)]
    managed_dir: Option<PathBuf>,
    #[arg(long = "mirror-prefix")]
    mirror_prefixes: Vec<String>,
    #[arg(long = "package-index")]
    package_indexes: Vec<String>,
    #[arg(long = "python-mirror")]
    python_install_mirrors: Vec<String>,
    #[arg(long)]
    gateway_base: Option<String>,
    #[arg(long)]
    country: Option<String>,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    max_download_bytes: Option<u64>,
    #[arg(long)]
    plan_file: Option<PathBuf>,
    #[arg(long)]
    method: Option<String>,
    #[arg(long)]
    id: Option<String>,
    #[arg(long = "tool-version")]
    tool_version: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    sha256: Option<String>,
    #[arg(long)]
    archive_binary: Option<String>,
    #[arg(long)]
    binary_name: Option<String>,
    #[arg(long)]
    destination: Option<String>,
    #[arg(long)]
    package: Option<String>,
    #[arg(long)]
    manager: Option<String>,
    #[arg(long)]
    python: Option<String>,
    #[arg(long, default_value_t = false)]
    json: bool,
    #[arg(long, default_value_t = false)]
    strict: bool,
}

impl BootstrapArgs {
    fn build_execution_request(&self) -> ExecutionRequest {
        ExecutionRequest {
            target_triple: self.target_triple.clone(),
            managed_dir: self.managed_dir.clone(),
            plan_base_dir: None,
            mirror_prefixes: self.mirror_prefixes.clone(),
            package_indexes: self.package_indexes.clone(),
            python_install_mirrors: self.python_install_mirrors.clone(),
            gateway_base: self.gateway_base.clone(),
            country: self.country.clone(),
            max_download_bytes: self.max_download_bytes,
        }
    }

    fn build_bootstrap_command(&self) -> BootstrapCommand {
        BootstrapCommand {
            execution: self.build_execution_request(),
            tools: self.tools.clone(),
        }
    }

    fn build_direct_plan(&self) -> Result<InstallPlan, InstallerError> {
        let id = self
            .id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                InstallerError::usage("`--id` is required when `--method` is provided")
            })?;
        let method = self
            .method
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| InstallerError::usage("`--method` cannot be empty"))?;
        Ok(InstallPlan {
            schema_version: Some(toolchain_installer::PLAN_SCHEMA_VERSION),
            items: vec![InstallPlanItem {
                id,
                method,
                version: self.tool_version.clone(),
                url: self.url.clone(),
                sha256: self.sha256.clone(),
                archive_binary: self.archive_binary.clone(),
                binary_name: self.binary_name.clone(),
                destination: self.destination.clone(),
                package: self.package.clone(),
                manager: self.manager.clone(),
                python: self.python.clone(),
            }],
        })
    }

    fn validate_mode_args(&self) -> Result<(), InstallerError> {
        if !self.tools.is_empty() {
            if self.method.is_some() {
                return Err(InstallerError::usage(
                    "`--tool` cannot be used with `--method`",
                ));
            }
            if self.plan_file.is_some() {
                return Err(InstallerError::usage(
                    "`--tool` cannot be used with `--plan-file`",
                ));
            }
        }

        if self.method.is_some() {
            if self.plan_file.is_some() {
                return Err(InstallerError::usage(
                    "`--method` and `--plan-file` cannot be used together",
                ));
            }
            return Ok(());
        }

        let direct_plan_flags = self.direct_plan_only_flags();
        if direct_plan_flags.is_empty() {
            return Ok(());
        }

        if self.plan_file.is_some() {
            return Err(InstallerError::usage(format!(
                "direct-plan flags cannot be used with `--plan-file`: {}",
                direct_plan_flags.join(", ")
            )));
        }

        Err(InstallerError::usage(format!(
            "direct-plan flags require `--method`: {}",
            direct_plan_flags.join(", ")
        )))
    }

    fn direct_plan_only_flags(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if self.id.is_some() {
            flags.push("--id");
        }
        if self.tool_version.is_some() {
            flags.push("--tool-version");
        }
        if self.url.is_some() {
            flags.push("--url");
        }
        if self.sha256.is_some() {
            flags.push("--sha256");
        }
        if self.archive_binary.is_some() {
            flags.push("--archive-binary");
        }
        if self.binary_name.is_some() {
            flags.push("--binary-name");
        }
        if self.destination.is_some() {
            flags.push("--destination");
        }
        if self.package.is_some() {
            flags.push("--package");
        }
        if self.manager.is_some() {
            flags.push("--manager");
        }
        if self.python.is_some() {
            flags.push("--python");
        }
        flags
    }
}

#[derive(Parser, Debug)]
#[command(version, about = "Reusable toolchain bootstrap installer")]
struct RootCli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Bootstrap(BootstrapArgs),
}

pub(crate) async fn run() -> Result<(), InstallerError> {
    let cli = RootCli::parse();
    let Commands::Bootstrap(args) = cli.command;
    args.validate_mode_args()?;
    let result = if args.method.is_some() {
        let execution_request = args.build_execution_request();
        let plan = args.build_direct_plan()?;
        validate_install_plan_with_request(&plan, &execution_request)?;
        toolchain_installer::apply_install_plan(&plan, &execution_request).await?
    } else if let Some(plan_file) = args.plan_file.as_ref() {
        let plan_base_dir = resolve_plan_base_dir(plan_file)?;
        let text = std::fs::read_to_string(plan_file).map_err(|err| {
            InstallerError::usage(format!(
                "read plan file `{}` failed: {err}",
                plan_file.display()
            ))
        })?;
        let plan: InstallPlan = serde_json::from_str(&text).map_err(|err| {
            InstallerError::usage(format!(
                "parse plan file `{}` failed: {err}",
                plan_file.display()
            ))
        })?;
        let mut execution_request = args.build_execution_request();
        execution_request.plan_base_dir = Some(plan_base_dir);
        validate_install_plan_with_request(&plan, &execution_request)?;
        toolchain_installer::apply_install_plan(&plan, &execution_request).await?
    } else {
        let command = args.build_bootstrap_command();
        toolchain_installer::bootstrap(&command).await?
    };

    if args.json {
        let value = serde_json::to_string_pretty(&result).expect("serialize installer result");
        println!("{value}");
    } else {
        println!(
            "result: host={} target={} managed_dir={}",
            result.host_triple, result.target_triple, result.managed_dir
        );
        for item in &result.items {
            let detail = item
                .detail
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            println!("- {}: {:?}{}", item.tool, item.status, detail);
        }
    }

    if args.strict && has_failure(&result.items) {
        std::process::exit(ExitCode::StrictFailure.as_i32());
    }
    if result.items.len() == 1
        && let Some(code) = result.items[0].failure_code
    {
        std::process::exit(code.as_i32());
    }
    Ok(())
}

fn resolve_plan_base_dir(plan_file: &Path) -> Result<PathBuf, InstallerError> {
    std::env::current_dir()
        .map_err(|err| InstallerError::usage(format!("resolve plan base directory failed: {err}")))
        .map(|cwd| resolve_plan_base_dir_from_cwd(plan_file, &cwd))
}

fn resolve_plan_base_dir_from_cwd(plan_file: &Path, cwd: &Path) -> PathBuf {
    let parent = plan_file
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent.is_absolute() {
        return normalize_plan_base_dir(parent);
    }
    normalize_plan_base_dir(&cwd.join(parent))
}

fn normalize_plan_base_dir(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let mut anchor_len = 0usize;
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if normalized.components().count() > anchor_len {
                    normalized.pop();
                } else if anchor_len == 0 {
                    normalized.push(component.as_os_str());
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                normalized.push(component.as_os_str());
                anchor_len = normalized.components().count();
            }
            std::path::Component::Normal(_) => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{BootstrapArgs, normalize_plan_base_dir, resolve_plan_base_dir_from_cwd};

    #[test]
    fn validate_mode_args_rejects_tool_with_method() {
        let args = BootstrapArgs {
            tools: vec!["git".to_string()],
            method: Some("pip".to_string()),
            ..BootstrapArgs::default()
        };

        let err = args.validate_mode_args().expect_err("should reject");
        assert_eq!(err.to_string(), "`--tool` cannot be used with `--method`");
    }

    #[test]
    fn validate_mode_args_rejects_tool_with_plan_file() {
        let args = BootstrapArgs {
            tools: vec!["git".to_string()],
            plan_file: Some("plan.json".into()),
            ..BootstrapArgs::default()
        };

        let err = args.validate_mode_args().expect_err("should reject");
        assert_eq!(
            err.to_string(),
            "`--tool` cannot be used with `--plan-file`"
        );
    }

    #[test]
    fn validate_mode_args_rejects_direct_plan_flags_without_method_even_with_tool() {
        let args = BootstrapArgs {
            tools: vec!["git".to_string()],
            package: Some("ruff".to_string()),
            ..BootstrapArgs::default()
        };

        let err = args.validate_mode_args().expect_err("should reject");
        assert_eq!(
            err.to_string(),
            "direct-plan flags require `--method`: --package"
        );
    }

    #[test]
    fn normalize_plan_base_dir_collapses_parent_components() {
        assert_eq!(
            normalize_plan_base_dir(Path::new("/repo/install-plans/../plans")),
            PathBuf::from("/repo/plans")
        );
    }

    #[test]
    fn resolve_plan_base_dir_collapses_parent_components_after_joining_cwd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let resolved =
            resolve_plan_base_dir_from_cwd(Path::new("./plans/../plans/demo.json"), temp.path());

        assert_eq!(resolved, temp.path().join("plans"));
    }

    #[test]
    fn build_direct_plan_maps_tool_version_into_generic_version_field() {
        let args = BootstrapArgs {
            method: Some("cargo_install".to_string()),
            id: Some("cargo-binstall".to_string()),
            tool_version: Some("1.7.0".to_string()),
            package: Some("cargo-binstall".to_string()),
            ..BootstrapArgs::default()
        };

        let plan = args.build_direct_plan().expect("direct plan");
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].version.as_deref(), Some("1.7.0"));
    }
}
