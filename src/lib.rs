use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 20;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapStatus {
    Present,
    Installed,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapItem {
    pub tool: String,
    pub status: BootstrapStatus,
    pub source: Option<String>,
    pub destination: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapResult {
    pub schema_version: u32,
    pub target_triple: String,
    pub managed_dir: String,
    pub items: Vec<BootstrapItem>,
}

#[derive(Debug, Clone, Default)]
pub struct BootstrapRequest {
    pub target_triple: Option<String>,
    pub managed_dir: Option<PathBuf>,
    pub tools: Vec<String>,
    pub mirror_prefixes: Vec<String>,
    pub gateway_base: Option<String>,
    pub country: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallPlan {
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub items: Vec<InstallPlanItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallPlanItem {
    pub id: String,
    pub method: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub archive_binary: Option<String>,
    pub binary_name: Option<String>,
    pub destination: Option<String>,
    pub package: Option<String>,
    pub manager: Option<String>,
    pub python: Option<String>,
}

#[derive(Debug, Clone)]
struct PublicBootstrapConfig {
    github_api_bases: Vec<String>,
    mirror_prefixes: Vec<String>,
    gateway_base: Option<String>,
    country: Option<String>,
    http_timeout: Duration,
}

impl PublicBootstrapConfig {
    fn from_request(request: &BootstrapRequest) -> Self {
        let github_api_bases = parse_csv_env("TOOLCHAIN_INSTALLER_GITHUB_API_BASES");
        let github_api_bases = if github_api_bases.is_empty() {
            vec![DEFAULT_GITHUB_API_BASE.to_string()]
        } else {
            github_api_bases
        };

        let mut mirror_prefixes = parse_csv_env("TOOLCHAIN_INSTALLER_MIRROR_PREFIXES");
        for prefix in &request.mirror_prefixes {
            if !prefix.trim().is_empty() {
                mirror_prefixes.push(prefix.trim().to_string());
            }
        }
        let mut unique = BTreeSet::new();
        mirror_prefixes.retain(|value| unique.insert(value.clone()));

        let gateway_base = request
            .gateway_base
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("TOOLCHAIN_INSTALLER_GATEWAY_BASE")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });

        let country = request
            .country
            .as_ref()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("TOOLCHAIN_INSTALLER_COUNTRY")
                    .ok()
                    .map(|value| value.trim().to_ascii_uppercase())
                    .filter(|value| !value.is_empty())
            });

        let http_timeout = std::env::var("TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS));

        Self {
            github_api_bases,
            mirror_prefixes,
            gateway_base,
            country,
            http_timeout,
        }
    }

    fn use_gateway_for_git_release(&self) -> bool {
        self.gateway_base.is_some() && self.country.as_deref() == Some("CN")
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub fn has_failure(items: &[BootstrapItem]) -> bool {
    items
        .iter()
        .any(|item| item.status == BootstrapStatus::Failed)
}

pub async fn bootstrap(request: &BootstrapRequest) -> anyhow::Result<BootstrapResult> {
    let target_triple = detect_target_triple(request.target_triple.as_deref())
        .ok_or_else(|| anyhow::anyhow!("unsupported platform/arch for target triple detection"))?;
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve managed toolchain directory"))?;
    let cfg = PublicBootstrapConfig::from_request(request);
    let client = reqwest::Client::builder()
        .timeout(cfg.http_timeout)
        .user_agent("toolchain-installer")
        .build()
        .context("build http client")?;
    let binary_ext = target_binary_ext(&target_triple);

    let tools = normalize_tools(&request.tools);
    let mut items = Vec::new();
    for tool in tools {
        let item = bootstrap_one_tool(
            tool.as_str(),
            &target_triple,
            binary_ext,
            &managed_dir,
            &cfg,
            &client,
        )
        .await;
        items.push(item);
    }

    Ok(BootstrapResult {
        schema_version: 1,
        target_triple,
        managed_dir: managed_dir.display().to_string(),
        items,
    })
}

pub async fn apply_install_plan(
    plan: &InstallPlan,
    request: &BootstrapRequest,
) -> anyhow::Result<BootstrapResult> {
    let target_triple = detect_target_triple(request.target_triple.as_deref())
        .ok_or_else(|| anyhow::anyhow!("unsupported platform/arch for target triple detection"))?;
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve managed toolchain directory"))?;
    let cfg = PublicBootstrapConfig::from_request(request);
    let client = reqwest::Client::builder()
        .timeout(cfg.http_timeout)
        .user_agent("toolchain-installer")
        .build()
        .context("build http client")?;

    let mut items = Vec::new();
    for item in &plan.items {
        items.push(
            execute_plan_item(item, &target_triple, &managed_dir, &cfg, &client)
                .await
                .unwrap_or_else(|err| BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Failed,
                    source: None,
                    destination: item.destination.clone(),
                    detail: Some(err.to_string()),
                }),
        );
    }

    Ok(BootstrapResult {
        schema_version: plan.schema_version.unwrap_or(1),
        target_triple,
        managed_dir: managed_dir.display().to_string(),
        items,
    })
}

fn normalize_tools(input: &[String]) -> Vec<String> {
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

async fn execute_plan_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> anyhow::Result<BootstrapItem> {
    let method = item.method.trim().to_ascii_lowercase();
    match method.as_str() {
        "release" => execute_release_item(item, target_triple, managed_dir, cfg, client).await,
        "apt" | "system_package" => execute_system_package_item(item),
        "pip" => execute_pip_item(item),
        _ => Ok(BootstrapItem {
            tool: item.id.clone(),
            status: BootstrapStatus::Unsupported,
            source: None,
            destination: item.destination.clone(),
            detail: Some(format!("unsupported plan method `{}`", item.method)),
        }),
    }
}

async fn execute_release_item(
    item: &InstallPlanItem,
    target_triple: &str,
    managed_dir: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> anyhow::Result<BootstrapItem> {
    let url = item
        .url
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("release method requires `url`"))?;
    let binary_name = item
        .binary_name
        .clone()
        .unwrap_or_else(|| format!("{}{}", item.id, target_binary_ext(target_triple)));
    let destination = item
        .destination
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| managed_dir.join(&binary_name));

    let gateway = if cfg.use_gateway_for_git_release() && item.id == "git" {
        infer_gateway_candidate_for_git_release(cfg, &url)
    } else {
        None
    };
    let (bytes, source) =
        download_with_candidates(client, &url, &cfg.mirror_prefixes, gateway.as_deref()).await?;

    if let Some(raw_sha) = item.sha256.as_deref() {
        let expected_sha = parse_sha256_user_input(raw_sha)
            .ok_or_else(|| anyhow::anyhow!("invalid sha256 value for `{}`", item.id))?;
        verify_sha256(&bytes, &expected_sha)?;
    }

    let asset_name = url
        .rsplit('/')
        .next()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.bin", item.id));
    if asset_name.ends_with(".zip")
        || asset_name.ends_with(".tar.gz")
        || asset_name.ends_with(".tar.xz")
    {
        install_binary_from_archive(
            &asset_name,
            &bytes,
            &binary_name,
            &item.id,
            &destination,
            item.archive_binary.as_deref(),
        )?;
    } else {
        let mut reader = Cursor::new(bytes);
        write_binary_from_reader(&mut reader, &destination)?;
    }

    Ok(BootstrapItem {
        tool: item.id.clone(),
        status: BootstrapStatus::Installed,
        source: Some(source),
        destination: Some(destination.display().to_string()),
        detail: None,
    })
}

fn infer_gateway_candidate_for_git_release(
    cfg: &PublicBootstrapConfig,
    url: &str,
) -> Option<String> {
    let base = cfg.gateway_base.as_deref()?;
    let marker = "/git-for-windows/git/releases/download/";
    let index = url.find(marker)?;
    let suffix = &url[(index + marker.len())..];
    let mut segments = suffix.split('/');
    let tag = segments.next()?;
    let asset = segments.next()?;
    if tag.is_empty() || asset.is_empty() {
        return None;
    }
    Some(make_gateway_asset_candidate(base, "git", tag, asset))
}

fn execute_system_package_item(item: &InstallPlanItem) -> anyhow::Result<BootstrapItem> {
    let package = item
        .package
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("system_package method requires `package`"))?;
    let recipes = if let Some(manager) = item
        .manager
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    {
        single_manager_recipe(&manager, &package)?
    } else {
        system_package_install_recipes(std::env::consts::OS, &package)
    };
    if recipes.is_empty() {
        anyhow::bail!("no available package manager recipe for `{package}`");
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        match run_recipe(&recipe.program, &recipe.args) {
            Ok(_) => {
                return Ok(BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Installed,
                    source: Some(format!("system:{}", recipe.program)),
                    destination: item.destination.clone(),
                    detail: None,
                });
            }
            Err(err) => errors.push(format!("{} failed: {err}", recipe.program)),
        }
    }
    anyhow::bail!("all package manager recipes failed: {}", errors.join(" | "))
}

fn single_manager_recipe(manager: &str, package: &str) -> anyhow::Result<Vec<SystemInstallRecipe>> {
    let recipe = match manager {
        "apt" | "apt-get" => SystemInstallRecipe {
            program: "apt-get".to_string(),
            args: vec!["install".to_string(), "-y".to_string(), package.to_string()],
        },
        "dnf" => SystemInstallRecipe {
            program: "dnf".to_string(),
            args: vec!["install".to_string(), "-y".to_string(), package.to_string()],
        },
        "yum" => SystemInstallRecipe {
            program: "yum".to_string(),
            args: vec!["install".to_string(), "-y".to_string(), package.to_string()],
        },
        "apk" => SystemInstallRecipe {
            program: "apk".to_string(),
            args: vec![
                "add".to_string(),
                "--no-cache".to_string(),
                package.to_string(),
            ],
        },
        "pacman" => SystemInstallRecipe {
            program: "pacman".to_string(),
            args: vec![
                "-Sy".to_string(),
                "--noconfirm".to_string(),
                package.to_string(),
            ],
        },
        "zypper" => SystemInstallRecipe {
            program: "zypper".to_string(),
            args: vec![
                "--non-interactive".to_string(),
                "install".to_string(),
                package.to_string(),
            ],
        },
        "brew" => SystemInstallRecipe {
            program: "brew".to_string(),
            args: vec!["install".to_string(), package.to_string()],
        },
        other => anyhow::bail!("unsupported manager `{other}`"),
    };
    Ok(vec![recipe])
}

fn execute_pip_item(item: &InstallPlanItem) -> anyhow::Result<BootstrapItem> {
    let package = item
        .package
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("pip method requires `package`"))?;
    let preferred_python = item
        .python
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".to_string());
    let candidates = if preferred_python == "python3" {
        vec!["python3".to_string(), "python".to_string()]
    } else {
        vec![preferred_python]
    };

    let mut errors = Vec::new();
    for python in candidates {
        if !command_exists(&python) {
            errors.push(format!("{python} not found"));
            continue;
        }
        let args = vec![
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            package.clone(),
        ];
        match run_recipe(&python, &args) {
            Ok(_) => {
                return Ok(BootstrapItem {
                    tool: item.id.clone(),
                    status: BootstrapStatus::Installed,
                    source: Some(format!("pip:{python}")),
                    destination: item.destination.clone(),
                    detail: None,
                });
            }
            Err(err) => errors.push(format!("{python} failed: {err}")),
        }
    }
    anyhow::bail!("all pip recipes failed: {}", errors.join(" | "))
}

fn parse_sha256_user_input(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(parsed) = parse_sha256_digest(Some(trimmed)) {
        return Some(parsed);
    }
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.len() == 64 && lowered.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Some(lowered);
    }
    None
}

async fn bootstrap_one_tool(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    managed_dir: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> BootstrapItem {
    if command_available(tool) {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Present,
            source: None,
            destination: None,
            detail: None,
        };
    }

    let destination = managed_dir.join(format!("{tool}{binary_ext}"));
    if destination.exists() {
        return BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Installed,
            source: Some("managed".to_string()),
            destination: Some(destination.display().to_string()),
            detail: Some("managed binary already exists".to_string()),
        };
    }

    match install_tool(tool, target_triple, binary_ext, &destination, cfg, client).await {
        Ok(source) => BootstrapItem {
            tool: tool.to_string(),
            status: BootstrapStatus::Installed,
            source: Some(source),
            destination: Some(destination.display().to_string()),
            detail: None,
        },
        Err(err) => {
            let status = if is_supported_tool(tool) {
                BootstrapStatus::Failed
            } else {
                BootstrapStatus::Unsupported
            };
            BootstrapItem {
                tool: tool.to_string(),
                status,
                source: None,
                destination: Some(destination.display().to_string()),
                detail: Some(err.to_string()),
            }
        }
    }
}

fn is_supported_tool(tool: &str) -> bool {
    matches!(tool, "git" | "gh")
}

async fn install_tool(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    destination: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> anyhow::Result<String> {
    match tool {
        "gh" => install_gh_from_public(target_triple, binary_ext, destination, cfg, client).await,
        "git" => install_git(target_triple, destination, cfg, client).await,
        _ => anyhow::bail!("unsupported tool `{tool}`"),
    }
}

async fn install_git(
    target_triple: &str,
    destination: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> anyhow::Result<String> {
    if target_triple == "x86_64-pc-windows-msvc" || target_triple == "aarch64-pc-windows-msvc" {
        return install_git_from_public(target_triple, destination, cfg, client).await;
    }

    install_git_from_system_package_manager(target_triple)
}

fn install_git_from_system_package_manager(target_triple: &str) -> anyhow::Result<String> {
    let recipes = system_package_install_recipes(std::env::consts::OS, "git");
    if recipes.is_empty() {
        anyhow::bail!(
            "git install for target `{target_triple}` requires package manager but none is supported on this OS"
        );
    }

    let mut errors = Vec::new();
    for recipe in recipes {
        match run_recipe(&recipe.program, &recipe.args) {
            Ok(_) => {
                if command_available("git") {
                    return Ok(format!("system:{}", recipe.program));
                }
                errors.push(format!(
                    "{} succeeded but `git --version` is still unavailable",
                    recipe.program
                ));
            }
            Err(err) => errors.push(format!("{} failed: {err}", recipe.program)),
        }
    }

    anyhow::bail!(
        "all system package manager recipes failed: {}",
        errors.join(" | ")
    );
}

#[derive(Debug)]
struct SystemInstallRecipe {
    program: String,
    args: Vec<String>,
}

fn system_package_install_recipes(os: &str, package: &str) -> Vec<SystemInstallRecipe> {
    match os {
        "linux" => vec![
            SystemInstallRecipe {
                program: "apt-get".to_string(),
                args: vec!["install".to_string(), "-y".to_string(), package.to_string()],
            },
            SystemInstallRecipe {
                program: "dnf".to_string(),
                args: vec!["install".to_string(), "-y".to_string(), package.to_string()],
            },
            SystemInstallRecipe {
                program: "yum".to_string(),
                args: vec!["install".to_string(), "-y".to_string(), package.to_string()],
            },
            SystemInstallRecipe {
                program: "apk".to_string(),
                args: vec![
                    "add".to_string(),
                    "--no-cache".to_string(),
                    package.to_string(),
                ],
            },
            SystemInstallRecipe {
                program: "pacman".to_string(),
                args: vec![
                    "-Sy".to_string(),
                    "--noconfirm".to_string(),
                    package.to_string(),
                ],
            },
            SystemInstallRecipe {
                program: "zypper".to_string(),
                args: vec![
                    "--non-interactive".to_string(),
                    "install".to_string(),
                    package.to_string(),
                ],
            },
        ],
        "macos" => vec![SystemInstallRecipe {
            program: "brew".to_string(),
            args: vec!["install".to_string(), package.to_string()],
        }],
        _ => Vec::new(),
    }
}

fn run_recipe(program: &str, args: &[String]) -> anyhow::Result<()> {
    if !command_exists(program) {
        anyhow::bail!("command not found");
    }

    let output = if should_use_sudo() && command_exists("sudo") {
        let mut cmd = Command::new("sudo");
        cmd.arg("-n").arg(program);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("run sudo -n {program}"))?
    } else {
        let mut cmd = Command::new(program);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("run {program}"))?
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!(
        "status={} stderr={} stdout={}",
        output.status, stderr, stdout
    );
    anyhow::bail!("{combined}");
}

fn should_use_sudo() -> bool {
    #[cfg(unix)]
    {
        match Command::new("id")
            .arg("-u")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim() != "0",
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn command_exists(command: &str) -> bool {
    let mut cmd = Command::new(command);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.status().is_ok()
}

async fn install_gh_from_public(
    target_triple: &str,
    binary_ext: &str,
    destination: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> anyhow::Result<String> {
    let suffix = gh_asset_suffix_for_target(target_triple).ok_or_else(|| {
        anyhow::anyhow!("gh public recipe unsupported on target `{target_triple}`")
    })?;
    let release = fetch_latest_github_release(client, cfg, "cli/cli").await?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(suffix))
        .ok_or_else(|| anyhow::anyhow!("cannot find gh release asset suffix `{suffix}`"))?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref())
        .ok_or_else(|| anyhow::anyhow!("missing sha256 digest in gh release metadata"))?;
    let (bytes, source) = download_with_candidates(
        client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
        None,
    )
    .await?;
    verify_sha256(&bytes, &expected_sha)?;
    let binary_name = format!("gh{binary_ext}");
    install_binary_from_archive(&asset.name, &bytes, &binary_name, "gh", destination, None)?;
    Ok(source)
}

async fn install_git_from_public(
    target_triple: &str,
    destination: &Path,
    cfg: &PublicBootstrapConfig,
    client: &reqwest::Client,
) -> anyhow::Result<String> {
    let release = fetch_latest_github_release(client, cfg, "git-for-windows/git").await?;
    let asset = select_mingit_asset_for_target(&release.assets, target_triple)
        .ok_or_else(|| anyhow::anyhow!("cannot find MinGit asset for target `{target_triple}`"))?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref()).ok_or_else(|| {
        anyhow::anyhow!("missing sha256 digest in git-for-windows release metadata")
    })?;
    let gateway = if cfg.use_gateway_for_git_release() {
        cfg.gateway_base
            .as_deref()
            .map(|base| make_gateway_asset_candidate(base, "git", &release.tag_name, &asset.name))
    } else {
        None
    };
    let (bytes, source) = download_with_candidates(
        client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
        gateway.as_deref(),
    )
    .await?;
    verify_sha256(&bytes, &expected_sha)?;
    install_binary_from_archive(&asset.name, &bytes, "git.exe", "git", destination, None)?;
    Ok(source)
}

fn make_gateway_asset_candidate(base: &str, tool: &str, tag: &str, asset_name: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    let safe_tag = tag.trim();
    format!("{trimmed}/toolchain/{tool}/{safe_tag}/{asset_name}")
}

fn select_mingit_asset_for_target<'a>(
    assets: &'a [GithubReleaseAsset],
    target_triple: &str,
) -> Option<&'a GithubReleaseAsset> {
    match target_triple {
        "x86_64-pc-windows-msvc" => assets
            .iter()
            .find(|asset| {
                asset.name.starts_with("MinGit-") && asset.name.ends_with("-busybox-64-bit.zip")
            })
            .or_else(|| {
                assets.iter().find(|asset| {
                    asset.name.starts_with("MinGit-")
                        && asset.name.ends_with("-64-bit.zip")
                        && !asset.name.contains("busybox")
                })
            }),
        "aarch64-pc-windows-msvc" => assets
            .iter()
            .find(|asset| asset.name.starts_with("MinGit-") && asset.name.ends_with("-arm64.zip")),
        _ => None,
    }
}

fn gh_asset_suffix_for_target(target_triple: &str) -> Option<&'static str> {
    match target_triple {
        "x86_64-unknown-linux-gnu" => Some("_linux_amd64.tar.gz"),
        "aarch64-unknown-linux-gnu" => Some("_linux_arm64.tar.gz"),
        "x86_64-apple-darwin" => Some("_macOS_amd64.zip"),
        "aarch64-apple-darwin" => Some("_macOS_arm64.zip"),
        "x86_64-pc-windows-msvc" => Some("_windows_amd64.zip"),
        "aarch64-pc-windows-msvc" => Some("_windows_arm64.zip"),
        _ => None,
    }
}

async fn fetch_latest_github_release(
    client: &reqwest::Client,
    cfg: &PublicBootstrapConfig,
    repo: &str,
) -> anyhow::Result<GithubRelease> {
    let mut errors = Vec::new();
    for base in &cfg.github_api_bases {
        let trimmed = base.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let url = format!("{trimmed}/repos/{repo}/releases/latest");
        match client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    errors.push(format!("{url} -> HTTP {}", resp.status()));
                    continue;
                }
                match resp.json::<GithubRelease>().await {
                    Ok(release) => return Ok(release),
                    Err(err) => errors.push(format!("{url} -> invalid json: {err}")),
                }
            }
            Err(err) => errors.push(format!("{url} -> {err}")),
        }
    }
    anyhow::bail!(
        "failed to fetch latest release metadata for {repo}: {}",
        errors.join(" | ")
    );
}

async fn download_with_candidates(
    client: &reqwest::Client,
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
) -> anyhow::Result<(Vec<u8>, String)> {
    let mut errors = Vec::new();
    for candidate in make_download_candidates(canonical_url, mirror_prefixes, gateway_candidate) {
        match client.get(&candidate).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    errors.push(format!("{candidate} -> HTTP {}", resp.status()));
                    continue;
                }
                match resp.bytes().await {
                    Ok(bytes) => return Ok((bytes.to_vec(), candidate)),
                    Err(err) => errors.push(format!("{candidate} -> read body failed: {err}")),
                }
            }
            Err(err) => errors.push(format!("{candidate} -> {err}")),
        }
    }
    anyhow::bail!(
        "all download candidates failed for {canonical_url}: {}",
        errors.join(" | ")
    );
}

fn make_download_candidates(
    canonical_url: &str,
    mirror_prefixes: &[String],
    gateway_candidate: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(gateway) = gateway_candidate {
        let trimmed = gateway.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
    }
    out.push(canonical_url.to_string());
    for raw_prefix in mirror_prefixes {
        let prefix = raw_prefix.trim();
        if prefix.is_empty() {
            continue;
        }
        let candidate = if prefix.contains("{url}") {
            prefix.replace("{url}", canonical_url)
        } else {
            format!("{prefix}{canonical_url}")
        };
        if !out.iter().any(|value| value == &candidate) {
            out.push(candidate);
        }
    }
    out
}

fn parse_csv_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_sha256_digest(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    let value = raw.strip_prefix("sha256:")?.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(value)
}

fn verify_sha256(content: &[u8], expected_hex: &str) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(content);
    let digest = hasher.finalize();
    let actual = digest
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect::<String>();
    if actual != expected_hex {
        anyhow::bail!("checksum mismatch: expected {expected_hex}, got {actual}");
    }
    Ok(())
}

fn install_binary_from_archive(
    asset_name: &str,
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &Path,
    archive_binary_hint: Option<&str>,
) -> anyhow::Result<()> {
    if asset_name.ends_with(".tar.gz") {
        install_from_tar_gz(content, binary_name, tool, destination, archive_binary_hint)
    } else if asset_name.ends_with(".tar.xz") {
        install_from_tar_xz(content, binary_name, tool, destination, archive_binary_hint)
    } else if asset_name.ends_with(".zip") {
        install_from_zip(content, binary_name, tool, destination, archive_binary_hint)
    } else {
        anyhow::bail!("unsupported archive type for `{asset_name}`");
    }
}

fn install_from_tar_gz(
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &Path,
    archive_binary_hint: Option<&str>,
) -> anyhow::Result<()> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(content));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        let path = entry
            .path()
            .context("read tar entry path")?
            .to_string_lossy()
            .replace('\\', "/");
        if is_binary_entry_match(&path, binary_name, tool, archive_binary_hint) {
            write_binary_from_reader(&mut entry, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary `{binary_name}` not found in tar archive");
}

fn install_from_tar_xz(
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &Path,
    archive_binary_hint: Option<&str>,
) -> anyhow::Result<()> {
    let decoder = xz2::read::XzDecoder::new(Cursor::new(content));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("read tar.xz entries")? {
        let mut entry = entry.context("read tar.xz entry")?;
        let path = entry
            .path()
            .context("read tar.xz entry path")?
            .to_string_lossy()
            .replace('\\', "/");
        if is_binary_entry_match(&path, binary_name, tool, archive_binary_hint) {
            write_binary_from_reader(&mut entry, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary `{binary_name}` not found in tar.xz archive");
}

fn install_from_zip(
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &Path,
    archive_binary_hint: Option<&str>,
) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(content))
        .context("open zip archive for tool bootstrap")?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("open zip entry #{index}"))?;
        if entry.is_dir() {
            continue;
        }
        let path = entry.name().replace('\\', "/");
        if is_binary_entry_match(&path, binary_name, tool, archive_binary_hint) {
            write_binary_from_reader(&mut entry, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary `{binary_name}` not found in zip archive");
}

fn is_binary_entry_match(
    path: &str,
    binary_name: &str,
    tool: &str,
    archive_binary_hint: Option<&str>,
) -> bool {
    if let Some(hint) = archive_binary_hint {
        let hint = hint.trim().trim_start_matches('/').replace('\\', "/");
        if !hint.is_empty() {
            return path == hint || path.ends_with(&format!("/{hint}"));
        }
    }
    if path.ends_with(&format!("/bin/{binary_name}")) {
        return true;
    }
    if tool == "git" && binary_name.eq_ignore_ascii_case("git.exe") {
        return path.ends_with("/cmd/git.exe")
            || path.ends_with("/mingw64/bin/git.exe")
            || path.ends_with("/usr/bin/git.exe")
            || path.ends_with("/bin/git.exe");
    }
    false
}

fn write_binary_from_reader(reader: &mut dyn Read, destination: &Path) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = std::fs::File::create(destination)
        .with_context(|| format!("create {}", destination.display()))?;
    std::io::copy(reader, &mut file).with_context(|| format!("write {}", destination.display()))?;
    file.flush()
        .with_context(|| format!("flush {}", destination.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(destination)
            .with_context(|| format!("stat {}", destination.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(destination, perms)
            .with_context(|| format!("chmod {}", destination.display()))?;
    }
    Ok(())
}

fn command_available(command: &str) -> bool {
    let mut cmd = Command::new(command);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.status() {
        Ok(_) => true,
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    }
}

fn target_binary_ext(target_triple: &str) -> &'static str {
    if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

fn detect_target_triple(override_target: Option<&str>) -> Option<String> {
    if let Some(raw) = override_target {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin".to_string()),
        ("macos", "x86_64") => Some("x86_64-apple-darwin".to_string()),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu".to_string()),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu".to_string()),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc".to_string()),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc".to_string()),
        _ => None,
    }
}

fn resolve_managed_toolchain_dir(
    override_dir: Option<&Path>,
    target_triple: &str,
) -> Option<PathBuf> {
    if let Some(override_dir) = override_dir {
        return Some(override_dir.to_path_buf());
    }
    if let Ok(raw) = std::env::var("TOOLCHAIN_INSTALLER_MANAGED_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))?;
    let mut out = PathBuf::from(home);
    out.push(".toolchain-installer");
    out.push(target_triple);
    out.push("bin");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parse_sha256_digest_accepts_valid_value() {
        let digest = parse_sha256_digest(Some(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ));
        assert_eq!(
            digest.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn parse_sha256_user_input_accepts_raw_hex() {
        let digest = parse_sha256_user_input(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        assert_eq!(
            digest.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn make_download_candidates_prefers_gateway() {
        let out = make_download_candidates(
            "https://github.com/org/repo/releases/download/v1/x.tar.gz",
            &["https://proxy.example/".to_string()],
            Some("https://gateway.example/toolchain/gh/v1/x.tar.gz"),
        );
        assert_eq!(out[0], "https://gateway.example/toolchain/gh/v1/x.tar.gz");
        assert_eq!(
            out[1],
            "https://github.com/org/repo/releases/download/v1/x.tar.gz"
        );
    }

    #[test]
    fn gateway_only_enabled_for_cn() {
        let cfg_cn = PublicBootstrapConfig {
            github_api_bases: vec![DEFAULT_GITHUB_API_BASE.to_string()],
            mirror_prefixes: Vec::new(),
            gateway_base: Some("https://gw.example".to_string()),
            country: Some("CN".to_string()),
            http_timeout: Duration::from_secs(5),
        };
        assert!(cfg_cn.use_gateway_for_git_release());

        let cfg_us = PublicBootstrapConfig {
            github_api_bases: vec![DEFAULT_GITHUB_API_BASE.to_string()],
            mirror_prefixes: Vec::new(),
            gateway_base: Some("https://gw.example".to_string()),
            country: Some("US".to_string()),
            http_timeout: Duration::from_secs(5),
        };
        assert!(!cfg_us.use_gateway_for_git_release());
    }

    #[test]
    fn infer_gateway_candidate_for_git_release_parses_release_url() {
        let cfg = PublicBootstrapConfig {
            github_api_bases: vec![DEFAULT_GITHUB_API_BASE.to_string()],
            mirror_prefixes: Vec::new(),
            gateway_base: Some("https://gw.example".to_string()),
            country: Some("CN".to_string()),
            http_timeout: Duration::from_secs(5),
        };
        let candidate = infer_gateway_candidate_for_git_release(
            &cfg,
            "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip",
        )
        .expect("candidate");
        assert_eq!(
            candidate,
            "https://gw.example/toolchain/git/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip"
        );
    }

    #[test]
    fn select_mingit_prefers_busybox_on_x64() {
        let assets = vec![
            GithubReleaseAsset {
                name: "MinGit-2.53.0-64-bit.zip".to_string(),
                browser_download_url: "https://example.invalid/a.zip".to_string(),
                digest: Some(
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        .to_string(),
                ),
            },
            GithubReleaseAsset {
                name: "MinGit-2.53.0-busybox-64-bit.zip".to_string(),
                browser_download_url: "https://example.invalid/b.zip".to_string(),
                digest: Some(
                    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        .to_string(),
                ),
            },
        ];
        let selected = select_mingit_asset_for_target(&assets, "x86_64-pc-windows-msvc")
            .expect("selected asset");
        assert_eq!(selected.name, "MinGit-2.53.0-busybox-64-bit.zip");
    }

    #[test]
    fn system_recipes_cover_linux() {
        let recipes = system_package_install_recipes("linux", "git");
        assert!(!recipes.is_empty());
        assert!(recipes.iter().any(|recipe| recipe.program == "apt-get"));
    }

    #[test]
    fn single_manager_recipe_rejects_unknown_manager() {
        let err = single_manager_recipe("unknown", "git").expect_err("manager should be rejected");
        assert!(err.to_string().contains("unsupported manager"));
    }

    #[tokio::test]
    async fn install_gh_from_mock_release_api() -> anyhow::Result<()> {
        let archive_name = "gh_9.9.9_linux_amd64.tar.gz";
        let archive_bytes = make_tar_gz_archive(&[(
            "gh_9.9.9_linux_amd64/bin/gh",
            b"#!/bin/sh\necho mock-gh\n".as_slice(),
            0o755,
        )])?;
        let digest = sha256_hex(&archive_bytes);

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let base = format!("http://{addr}");
        let release_body = serde_json::json!({
            "tag_name": "v9.9.9",
            "assets": [{
                "name": archive_name,
                "browser_download_url": format!("{base}/asset/{archive_name}"),
                "digest": format!("sha256:{digest}")
            }]
        })
        .to_string()
        .into_bytes();

        let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
        routes.insert(
            "/api/repos/cli/cli/releases/latest".to_string(),
            release_body,
        );
        routes.insert(format!("/asset/{archive_name}"), archive_bytes);
        let handle = spawn_mock_http_server(listener, routes, 2);

        let cfg = PublicBootstrapConfig {
            github_api_bases: vec![format!("{base}/api")],
            mirror_prefixes: Vec::new(),
            gateway_base: None,
            country: None,
            http_timeout: Duration::from_secs(5),
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        let tmp = tempfile::tempdir()?;
        let destination = tmp.path().join("gh");

        let source =
            install_gh_from_public("x86_64-unknown-linux-gnu", "", &destination, &cfg, &client)
                .await?;
        assert_eq!(source, format!("{base}/asset/{archive_name}"));
        let installed = std::fs::read_to_string(&destination)?;
        assert!(installed.contains("mock-gh"));

        handle.join().expect("mock server thread join");
        Ok(())
    }

    #[test]
    fn install_binary_from_tar_xz_uses_hint() -> anyhow::Result<()> {
        let archive = make_tar_xz_archive(&[(
            "node-v1.0.0-linux-x64/bin/node",
            b"mock-node".as_slice(),
            0o755,
        )])?;
        let tmp = tempfile::tempdir()?;
        let destination = tmp.path().join("node");
        install_binary_from_archive(
            "node-v1.0.0-linux-x64.tar.xz",
            &archive,
            "node",
            "node",
            &destination,
            Some("node-v1.0.0-linux-x64/bin/node"),
        )?;
        let content = std::fs::read_to_string(&destination)?;
        assert_eq!(content, "mock-node");
        Ok(())
    }

    fn sha256_hex(content: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content);
        let digest = hasher.finalize();
        digest
            .iter()
            .map(|value| format!("{value:02x}"))
            .collect::<String>()
    }

    fn make_tar_gz_archive(entries: &[(&str, &[u8], u32)]) -> anyhow::Result<Vec<u8>> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, body, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, *path, &mut Cursor::new(*body))
                .with_context(|| format!("append tar entry {path}"))?;
        }
        let encoder = builder.into_inner().context("finalize tar builder")?;
        let archive = encoder.finish().context("finalize gzip stream")?;
        Ok(archive)
    }

    fn make_tar_xz_archive(entries: &[(&str, &[u8], u32)]) -> anyhow::Result<Vec<u8>> {
        let encoder = xz2::write::XzEncoder::new(Vec::new(), 6);
        let mut builder = tar::Builder::new(encoder);
        for (path, body, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, *path, &mut Cursor::new(*body))
                .with_context(|| format!("append tar.xz entry {path}"))?;
        }
        let encoder = builder.into_inner().context("finalize tar.xz builder")?;
        let archive = encoder.finish().context("finalize xz stream")?;
        Ok(archive)
    }

    fn spawn_mock_http_server(
        listener: TcpListener,
        routes: HashMap<String, Vec<u8>>,
        expected_requests: usize,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            for _ in 0..expected_requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buffer = [0_u8; 8192];
                let Ok(size) = stream.read(&mut buffer) else {
                    continue;
                };
                if size == 0 {
                    continue;
                }
                let request = String::from_utf8_lossy(&buffer[..size]);
                let request_line = request.lines().next().unwrap_or_default();
                let path = request_line.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = if let Some(body) = routes.get(path) {
                    ("200 OK", body.clone())
                } else {
                    ("404 Not Found", b"not found".to_vec())
                };
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        })
    }
}
