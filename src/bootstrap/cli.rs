use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use toolchain_installer::{
    BootstrapRequest, ExitCode, InstallPlan, InstallPlanItem, InstallerError, has_failure,
    validate_install_plan,
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
    fn build_request(&self) -> BootstrapRequest {
        BootstrapRequest {
            target_triple: self.target_triple.clone(),
            managed_dir: self.managed_dir.clone(),
            tools: self.tools.clone(),
            mirror_prefixes: self.mirror_prefixes.clone(),
            package_indexes: self.package_indexes.clone(),
            python_install_mirrors: self.python_install_mirrors.clone(),
            gateway_base: self.gateway_base.clone(),
            country: self.country.clone(),
            max_download_bytes: self.max_download_bytes,
        }
    }

    fn build_direct_plan(&self) -> Result<InstallPlan, InstallerError> {
        if self.plan_file.is_some() {
            return Err(InstallerError::usage(
                "`--method` and `--plan-file` cannot be used together",
            ));
        }
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
}

#[derive(Parser, Debug)]
#[command(version, about = "Reusable toolchain bootstrap installer")]
struct LegacyCli {
    #[command(flatten)]
    args: BootstrapArgs,
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
    let args = parse_bootstrap_args();
    let request = args.build_request();
    let result = if args.method.is_some() {
        let plan = args.build_direct_plan()?;
        validate_install_plan(&plan, request.target_triple.as_deref())?;
        toolchain_installer::apply_install_plan(&plan, &request).await?
    } else if let Some(plan_file) = args.plan_file.as_ref() {
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
        validate_install_plan(&plan, request.target_triple.as_deref())?;
        toolchain_installer::apply_install_plan(&plan, &request).await?
    } else {
        toolchain_installer::bootstrap(&request).await?
    };

    if args.json {
        let value = serde_json::to_string_pretty(&result).expect("serialize bootstrap result");
        println!("{value}");
    } else {
        println!(
            "bootstrap: host={} target={} managed_dir={}",
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

fn parse_bootstrap_args() -> BootstrapArgs {
    let argv: Vec<OsString> = std::env::args_os().collect();
    if matches!(
        argv.get(1).and_then(|value| value.to_str()),
        Some("bootstrap")
    ) {
        let cli = RootCli::parse_from(argv);
        match cli.command {
            Commands::Bootstrap(args) => args,
        }
    } else {
        LegacyCli::parse_from(argv).args
    }
}
