use std::path::PathBuf;

use clap::Parser;
use toolchain_installer::{BootstrapRequest, InstallPlan, InstallPlanItem, has_failure};

#[derive(Parser, Debug)]
#[command(version, about = "Reusable git/gh bootstrap installer")]
struct Cli {
    #[arg(long = "tool")]
    tools: Vec<String>,
    #[arg(long)]
    target_triple: Option<String>,
    #[arg(long)]
    managed_dir: Option<PathBuf>,
    #[arg(long = "mirror-prefix")]
    mirror_prefixes: Vec<String>,
    #[arg(long)]
    gateway_base: Option<String>,
    #[arg(long)]
    country: Option<String>,
    #[arg(long)]
    plan_file: Option<PathBuf>,
    #[arg(long)]
    method: Option<String>,
    #[arg(long)]
    id: Option<String>,
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

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let request = BootstrapRequest {
        target_triple: cli.target_triple,
        managed_dir: cli.managed_dir,
        tools: cli.tools,
        mirror_prefixes: cli.mirror_prefixes,
        gateway_base: cli.gateway_base,
        country: cli.country,
    };
    let result = if cli.method.is_some() {
        if cli.plan_file.is_some() {
            anyhow::bail!("`--method` and `--plan-file` cannot be used together");
        }
        let id = cli
            .id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("`--id` is required when `--method` is provided"))?;
        let method = cli
            .method
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("`--method` cannot be empty"))?;
        let plan = InstallPlan {
            schema_version: Some(1),
            items: vec![InstallPlanItem {
                id,
                method,
                url: cli.url,
                sha256: cli.sha256,
                archive_binary: cli.archive_binary,
                binary_name: cli.binary_name,
                destination: cli.destination,
                package: cli.package,
                manager: cli.manager,
                python: cli.python,
            }],
        };
        toolchain_installer::apply_install_plan(&plan, &request).await?
    } else if let Some(plan_file) = cli.plan_file {
        let text = std::fs::read_to_string(&plan_file).map_err(|err| {
            anyhow::anyhow!("read plan file `{}` failed: {err}", plan_file.display())
        })?;
        let plan: InstallPlan = serde_json::from_str(&text).map_err(|err| {
            anyhow::anyhow!("parse plan file `{}` failed: {err}", plan_file.display())
        })?;
        toolchain_installer::apply_install_plan(&plan, &request).await?
    } else {
        toolchain_installer::bootstrap(&request).await?
    };

    if cli.json {
        let value = serde_json::to_string_pretty(&result).expect("serialize bootstrap result");
        println!("{value}");
    } else {
        println!(
            "bootstrap: target={} managed_dir={}",
            result.target_triple, result.managed_dir
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

    if cli.strict && has_failure(&result.items) {
        std::process::exit(5);
    }
    Ok(())
}
