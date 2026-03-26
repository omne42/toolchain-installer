use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const DOC_PATH: &str = "docs/architecture/source-layout.md";
const SRC_DIR: &str = "src";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let staged = parse_args()?;
    let repo_root = repo_root()?;

    let (doc_text, source_files, snapshot_name) = if staged {
        (
            read_staged_text(&repo_root, DOC_PATH)?,
            list_staged_source_files(&repo_root)?,
            "staged index",
        )
    } else {
        (
            fs::read_to_string(repo_root.join(DOC_PATH)).map_err(|error| {
                format!(
                    "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not read `{DOC_PATH}` from the working tree\n\
- io error:\n{error}\n"
                )
            })?,
            list_worktree_source_files(&repo_root)?,
            "working tree",
        )
    };
    let source_snapshots = load_source_snapshots(&repo_root, &source_files, staged)?;

    let documented_files = collect_documented_source_files(&doc_text);
    let source_file_set = source_files.iter().cloned().collect::<BTreeSet<_>>();
    let documented_file_set = documented_files.iter().cloned().collect::<BTreeSet<_>>();
    let documented_top_level_dirs = collect_documented_top_level_src_dirs(&doc_text);
    let actual_top_level_dirs = collect_actual_top_level_src_dirs(&source_files);
    let expected_top_level_dirs = actual_top_level_dirs.clone();
    let actual_architecture_modules = collect_actual_architecture_modules(&source_files);
    let known_architecture_modules = known_architecture_modules();
    let missing_dependency_policy_modules = actual_architecture_modules
        .iter()
        .filter(|module| !known_architecture_modules.contains(module.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let stale_dependency_policy_modules = known_architecture_modules
        .iter()
        .filter(|module| !actual_architecture_modules.contains(**module))
        .map(|module| (*module).to_string())
        .collect::<Vec<_>>();
    let unknown_policy_dependencies =
        collect_unknown_policy_dependencies(&known_architecture_modules);

    let source_file_count = source_files.len();
    let missing_files = source_files
        .iter()
        .filter(|path| !documented_file_set.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let extra_documented_files = documented_files
        .iter()
        .filter(|path| !source_file_set.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let missing_top_level_dirs = actual_top_level_dirs
        .iter()
        .filter(|path| !documented_top_level_dirs.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let extra_documented_top_level_dirs = documented_top_level_dirs
        .iter()
        .filter(|path| !actual_top_level_dirs.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let dependency_violations = collect_dependency_violations(&source_snapshots);

    if !missing_files.is_empty()
        || !extra_documented_files.is_empty()
        || !missing_top_level_dirs.is_empty()
        || !extra_documented_top_level_dirs.is_empty()
        || documented_top_level_dirs != expected_top_level_dirs
        || !missing_dependency_policy_modules.is_empty()
        || !stale_dependency_policy_modules.is_empty()
        || !unknown_policy_dependencies.is_empty()
        || !dependency_violations.is_empty()
    {
        return Err(format_validation_failure(
            snapshot_name,
            &missing_files,
            &extra_documented_files,
            &missing_top_level_dirs,
            &extra_documented_top_level_dirs,
            &documented_top_level_dirs,
            &expected_top_level_dirs,
            &missing_dependency_policy_modules,
            &stale_dependency_policy_modules,
            &unknown_policy_dependencies,
            &dependency_violations,
        ));
    }

    println!(
        "source-layout validation passed for {} source files in the {}",
        source_file_count, snapshot_name
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceSnapshot {
    path: String,
    contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DependencyViolation {
    source_path: String,
    source_module: String,
    referenced_module: String,
}

fn parse_args() -> Result<bool, String> {
    let mut staged = false;
    let args = env::args().skip(1).collect::<Vec<_>>();
    for arg in args {
        match arg.as_str() {
            "--staged" => staged = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(format!(
                    "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- unsupported argument `{other}`\n\
- supported arguments: `--staged`, `--help`\n"
                ));
            }
        }
    }
    Ok(staged)
}

fn print_help() {
    println!("Verify source-layout.md consistency and top-level crate dependency direction.");
    println!();
    println!("Usage:");
    println!("  source-layout-check [--staged]");
}

fn repo_root() -> Result<PathBuf, String> {
    let current_dir = env::current_dir().map_err(|error| {
        format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not resolve current working directory\n\
- io error:\n{error}\n"
        )
    })?;

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&current_dir)
        .output()
        .map_err(|error| {
            format!(
                "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not execute `git rev-parse --show-toplevel`\n\
- io error:\n{error}\n"
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not resolve the repository root with `git rev-parse --show-toplevel`\n\
- git stderr:\n{}\n",
            render_bytes(&output.stderr)
        ));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- `git rev-parse --show-toplevel` returned non-UTF-8 output\n\
- utf-8 error:\n{error}\n"
        )
    })?;
    let root = stdout.trim();
    if root.is_empty() {
        return Err("source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- `git rev-parse --show-toplevel` returned an empty repository root\n"
            .to_string());
    }

    Ok(PathBuf::from(root))
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            format!(
                "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not execute `git {}`\n\
- io error:\n{error}\n",
                args.join(" ")
            )
        })?;
    Ok((
        output.status.code().unwrap_or(1),
        output.stdout,
        output.stderr,
    ))
}

fn read_staged_text(repo_root: &Path, relative_path: &str) -> Result<String, String> {
    let (status, stdout, stderr) = run_git(repo_root, &["show", &format!(":{relative_path}")])?;
    if status != 0 {
        return Err(format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not read staged file `{relative_path}` from the git index\n\
- git stderr:\n{}\n",
            render_bytes(&stderr)
        ));
    }
    String::from_utf8(stdout).map_err(|error| {
        format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- staged file `{relative_path}` is not valid UTF-8\n\
- utf-8 error:\n{error}\n"
        )
    })
}

fn read_worktree_text(repo_root: &Path, relative_path: &str) -> Result<String, String> {
    fs::read_to_string(repo_root.join(relative_path)).map_err(|error| {
        format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not read source file `{relative_path}` from the working tree\n\
- io error:\n{error}\n"
        )
    })
}

fn list_staged_source_files(repo_root: &Path) -> Result<Vec<String>, String> {
    let (status, stdout, stderr) =
        run_git(repo_root, &["ls-files", "-z", "--cached", "--", SRC_DIR])?;
    if status != 0 {
        return Err(format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not list staged files under `{SRC_DIR}/`\n\
- git stderr:\n{}\n",
            render_bytes(&stderr)
        ));
    }

    let mut paths = stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .filter(|path| path == &SRC_DIR || path.starts_with("src/"))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn list_worktree_source_files(repo_root: &Path) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    collect_worktree_files(&repo_root.join(SRC_DIR), repo_root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn load_source_snapshots(
    repo_root: &Path,
    source_files: &[String],
    staged: bool,
) -> Result<Vec<SourceSnapshot>, String> {
    source_files
        .iter()
        .map(|path| {
            let contents = if staged {
                read_staged_text(repo_root, path)?
            } else {
                read_worktree_text(repo_root, path)?
            };
            Ok(SourceSnapshot {
                path: path.clone(),
                contents,
            })
        })
        .collect()
}

fn collect_worktree_files(
    current_dir: &Path,
    repo_root: &Path,
    paths: &mut Vec<String>,
) -> Result<(), String> {
    if !current_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(current_dir).map_err(|error| {
        format!(
            "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not read directory `{}`\n\
- io error:\n{error}\n",
            normalize_path(current_dir, repo_root)
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not iterate directory `{}`\n\
- io error:\n{error}\n",
                normalize_path(current_dir, repo_root)
            )
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "source-layout validation failed\n\n\
Harness-style explicit disclosure:\n\
- could not inspect file type for `{}`\n\
- io error:\n{error}\n",
                normalize_path(&path, repo_root)
            )
        })?;
        if file_type.is_dir() {
            collect_worktree_files(&path, repo_root, paths)?;
            continue;
        }
        if file_type.is_file() {
            paths.push(normalize_path(&path, repo_root));
        }
    }

    Ok(())
}

fn collect_documented_source_files(doc_text: &str) -> Vec<String> {
    let mut documented = Vec::new();
    let mut directory_stack: Vec<(usize, String)> = Vec::new();

    for line in doc_text.lines() {
        let stripped = line.trim_start();
        if !stripped.starts_with("- `") {
            continue;
        }
        let Some(token) = extract_token(stripped) else {
            continue;
        };

        let indent = normalize_indent(&line[..line.len() - stripped.len()]);
        while directory_stack
            .last()
            .is_some_and(|(last_indent, _)| *last_indent >= indent)
        {
            directory_stack.pop();
        }

        if token.ends_with('/') {
            if token.starts_with("src/") {
                directory_stack.push((indent, token.trim_end_matches('/').to_string()));
            } else if let Some((_, parent)) = directory_stack.last() {
                directory_stack.push((
                    indent,
                    format!("{}/{}", parent, token.trim_end_matches('/')),
                ));
            }
            continue;
        }

        if token.starts_with("src/") {
            if token == SRC_DIR || token.starts_with("src/") {
                documented.push(token.to_string());
            }
            continue;
        }

        if let Some((_, parent)) = directory_stack.last() {
            documented.push(format!("{parent}/{token}"));
        }
    }

    documented.sort();
    documented.dedup();
    documented
}

fn collect_documented_top_level_src_dirs(doc_text: &str) -> Vec<String> {
    let mut documented_dirs = Vec::new();
    for line in doc_text.lines() {
        let stripped = line.trim_start();
        if !stripped.starts_with("- `") {
            continue;
        }
        let Some(token) = extract_token(stripped) else {
            continue;
        };
        if !token.starts_with("src/") || !token.ends_with('/') {
            continue;
        }
        let trimmed = token.trim_end_matches('/');
        if top_level_src_dir(trimmed) {
            documented_dirs.push(trimmed.to_string());
        }
    }
    documented_dirs
}

fn collect_actual_top_level_src_dirs(source_files: &[String]) -> Vec<String> {
    let mut dirs = source_files
        .iter()
        .filter_map(|path| {
            path.strip_prefix("src/")
                .and_then(|rest| rest.split_once('/'))
                .map(|(dir_name, _)| format!("src/{dir_name}"))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

fn architecture_module_for_path(path: &str) -> Option<String> {
    match path {
        "src/main.rs" | "src/lib.rs" | "src/lib_tests.rs" | "src/cli.rs" => None,
        _ => path.strip_prefix("src/").and_then(|rest| {
            if let Some((module, _)) = rest.split_once('/') {
                Some(module.to_string())
            } else {
                rest.strip_suffix(".rs").map(|module| module.to_string())
            }
        }),
    }
}

fn collect_actual_architecture_modules(source_files: &[String]) -> BTreeSet<String> {
    source_files
        .iter()
        .filter_map(|path| architecture_module_for_path(path))
        .collect()
}

fn allowed_dependencies_for(module: &str) -> &'static [&'static str] {
    match module {
        "application" => &[
            "artifact",
            "builtin_tools",
            "contracts",
            "error",
            "install_plan",
            "installer_runtime_config",
            "managed_toolchain",
        ],
        "artifact" => &["contracts"],
        "builtin_tools" => &[
            "artifact",
            "contracts",
            "download_sources",
            "error",
            "external_gateway",
            "installer_runtime_config",
            "managed_toolchain",
            "plan_items",
        ],
        "contracts" => &["error"],
        "download_sources" => &["contracts"],
        "error" => &[],
        "external_gateway" => &["installer_runtime_config"],
        "installer_runtime_config" => &["contracts"],
        "managed_toolchain" => &[
            "artifact",
            "contracts",
            "download_sources",
            "error",
            "installer_runtime_config",
            "plan_items",
        ],
        "install_plan" => &[
            "contracts",
            "download_sources",
            "error",
            "external_gateway",
            "installer_runtime_config",
            "managed_toolchain",
            "plan_items",
        ],
        "plan_items" => &[],
        _ => &[],
    }
}

fn known_architecture_modules() -> BTreeSet<&'static str> {
    [
        "application",
        "artifact",
        "builtin_tools",
        "contracts",
        "download_sources",
        "error",
        "external_gateway",
        "install_plan",
        "installer_runtime_config",
        "managed_toolchain",
        "plan_items",
    ]
    .into_iter()
    .collect()
}

fn collect_unknown_policy_dependencies(known_modules: &BTreeSet<&'static str>) -> Vec<String> {
    let mut unknown_dependencies = Vec::new();
    for module in known_modules {
        for dependency in allowed_dependencies_for(module) {
            if !known_modules.contains(dependency) {
                unknown_dependencies.push((*dependency).to_string());
            }
        }
    }
    unknown_dependencies.sort();
    unknown_dependencies.dedup();
    unknown_dependencies
}

fn collect_dependency_violations(source_snapshots: &[SourceSnapshot]) -> Vec<DependencyViolation> {
    let known_modules = known_architecture_modules();
    let mut violations = Vec::new();

    for snapshot in source_snapshots {
        let Some(source_module) = architecture_module_for_path(&snapshot.path) else {
            continue;
        };
        if !known_modules.contains(source_module.as_str()) {
            continue;
        }
        let allowed = allowed_dependencies_for(&source_module)
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let referenced_modules =
            extract_crate_module_dependencies(&snapshot.contents, &known_modules);
        for referenced_module in referenced_modules {
            if referenced_module == source_module || allowed.contains(referenced_module.as_str()) {
                continue;
            }
            violations.push(DependencyViolation {
                source_path: snapshot.path.clone(),
                source_module: source_module.clone(),
                referenced_module,
            });
        }
    }

    violations.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then(left.referenced_module.cmp(&right.referenced_module))
    });
    violations
}

fn extract_crate_module_dependencies(
    contents: &str,
    known_modules: &BTreeSet<&str>,
) -> BTreeSet<String> {
    let mut dependencies = BTreeSet::new();
    let mut remaining = contents;
    while let Some(position) = remaining.find("crate::") {
        let after = &remaining[position + "crate::".len()..];
        let module = after
            .chars()
            .take_while(|character| character.is_ascii_lowercase() || *character == '_')
            .collect::<String>();
        if known_modules.contains(module.as_str()) {
            dependencies.insert(module);
        }
        remaining = &after[after
            .chars()
            .take_while(|character| character.is_ascii_lowercase() || *character == '_')
            .map(char::len_utf8)
            .sum::<usize>()..];
    }
    dependencies
}

fn top_level_src_dir(path: &str) -> bool {
    path.strip_prefix("src/")
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
}

fn extract_token(stripped_line: &str) -> Option<&str> {
    let remainder = stripped_line.strip_prefix("- `")?;
    let end = remainder.find('`')?;
    Some(&remainder[..end])
}

fn normalize_indent(raw_indent: &str) -> usize {
    raw_indent
        .chars()
        .map(|character| if character == '\t' { 4 } else { 1 })
        .sum()
}

fn format_validation_failure(
    snapshot_name: &str,
    missing_paths: &[String],
    extra_documented_files: &[String],
    missing_top_level_dirs: &[String],
    extra_documented_top_level_dirs: &[String],
    documented_top_level_dirs: &[String],
    expected_top_level_dirs: &[String],
    missing_dependency_policy_modules: &[String],
    stale_dependency_policy_modules: &[String],
    unknown_policy_dependencies: &[String],
    dependency_violations: &[DependencyViolation],
) -> String {
    let mut message =
        String::from("source-layout validation failed\n\nHarness-style explicit disclosure:\n");
    if !missing_paths.is_empty() {
        message.push_str(&format!(
            "- `{DOC_PATH}` does not cover every file under `{SRC_DIR}/` in the {snapshot_name}\n"
        ));
        message.push_str(&format!("- missing file count: {}\n", missing_paths.len()));
    }
    if !extra_documented_files.is_empty() {
        message.push_str(&format!(
            "- `{DOC_PATH}` still contains `src/...` file entries that do not exist in the {snapshot_name}\n"
        ));
        message.push_str(&format!(
            "- extra documented file count: {}\n",
            extra_documented_files.len()
        ));
    }
    if !missing_top_level_dirs.is_empty() {
        message.push_str(&format!(
            "- some top-level `src/*/` directories exist in the {snapshot_name} but are not documented\n"
        ));
        message.push_str(&format!(
            "- missing top-level directory count: {}\n",
            missing_top_level_dirs.len()
        ));
    }
    if !extra_documented_top_level_dirs.is_empty() {
        message.push_str(&format!(
            "- some documented top-level `src/*/` directories do not exist in the {snapshot_name}\n"
        ));
        message.push_str(&format!(
            "- extra documented top-level directory count: {}\n",
            extra_documented_top_level_dirs.len()
        ));
    }
    if documented_top_level_dirs != expected_top_level_dirs {
        message.push_str(
            "- top-level `src/*/` directory entries in `docs/architecture/source-layout.md` are not ordered alphabetically\n",
        );
    }
    if !missing_dependency_policy_modules.is_empty() {
        message.push_str(
            "- some top-level source modules exist in `src/` but do not have an explicit dependency policy entry\n",
        );
        message.push_str(&format!(
            "- missing dependency-policy module count: {}\n",
            missing_dependency_policy_modules.len()
        ));
    }
    if !stale_dependency_policy_modules.is_empty() {
        message.push_str(
            "- the dependency policy still contains top-level modules that no longer exist in `src/`\n",
        );
        message.push_str(&format!(
            "- stale dependency-policy module count: {}\n",
            stale_dependency_policy_modules.len()
        ));
    }
    if !unknown_policy_dependencies.is_empty() {
        message.push_str(
            "- the dependency policy references top-level modules that are not defined in the policy set\n",
        );
        message.push_str(&format!(
            "- unknown dependency-policy target count: {}\n",
            unknown_policy_dependencies.len()
        ));
    }
    if !dependency_violations.is_empty() {
        message.push_str(
            "- top-level crate module dependencies violate the enforced architecture direction policy\n",
        );
        message.push_str(&format!(
            "- dependency violation count: {}\n",
            dependency_violations.len()
        ));
    }

    if !missing_paths.is_empty() {
        message.push_str("\nMissing file entries:\n");
        for path in missing_paths {
            message.push_str(&format!("- `{path}`\n"));
        }
    }
    if !extra_documented_files.is_empty() {
        message.push_str("\nExtra documented file entries:\n");
        for path in extra_documented_files {
            message.push_str(&format!("- `{path}`\n"));
        }
    }
    if !missing_top_level_dirs.is_empty() {
        message.push_str("\nMissing top-level src directory entries:\n");
        for path in missing_top_level_dirs {
            message.push_str(&format!("- `{path}/`\n"));
        }
    }
    if !extra_documented_top_level_dirs.is_empty() {
        message.push_str("\nExtra documented top-level src directory entries:\n");
        for path in extra_documented_top_level_dirs {
            message.push_str(&format!("- `{path}/`\n"));
        }
    }

    if documented_top_level_dirs != expected_top_level_dirs {
        message.push_str("\nDocumented top-level src directory order:\n");
        for path in documented_top_level_dirs {
            message.push_str(&format!("- `{path}/`\n"));
        }
        message.push_str("\nExpected alphabetical order:\n");
        for path in expected_top_level_dirs {
            message.push_str(&format!("- `{path}/`\n"));
        }
    }
    if !missing_dependency_policy_modules.is_empty() {
        message.push_str("\nTop-level modules missing dependency policy entries:\n");
        for module in missing_dependency_policy_modules {
            message.push_str(&format!("- `{module}`\n"));
        }
    }
    if !stale_dependency_policy_modules.is_empty() {
        message.push_str("\nStale dependency policy modules:\n");
        for module in stale_dependency_policy_modules {
            message.push_str(&format!("- `{module}`\n"));
        }
    }
    if !unknown_policy_dependencies.is_empty() {
        message.push_str("\nUnknown dependency policy targets:\n");
        for module in unknown_policy_dependencies {
            message.push_str(&format!("- `{module}`\n"));
        }
    }
    if !dependency_violations.is_empty() {
        message.push_str("\nDependency direction violations:\n");
        for violation in dependency_violations {
            let allowed = allowed_dependencies_for(&violation.source_module).join(", ");
            message.push_str(&format!(
                "- `{}` imports `{}` from `{}`; allowed top-level dependencies: [{}]\n",
                violation.source_path,
                violation.referenced_module,
                violation.source_module,
                allowed
            ));
        }
    }

    message.push_str("\nRequired action:\n");
    if !missing_paths.is_empty() {
        message.push_str(
            "- add each missing file to `docs/architecture/source-layout.md` under the correct directory section\n",
        );
        message.push_str(
            "- describe the file responsibility explicitly instead of leaving the architecture map stale\n",
        );
    }
    if !extra_documented_files.is_empty() {
        message.push_str(
            "- remove or rename every stale `src/...` file entry that no longer matches the real source tree\n",
        );
    }
    if !missing_top_level_dirs.is_empty() {
        message.push_str(
            "- add every missing top-level `src/*/` directory section that is still present in the real source tree\n",
        );
    }
    if !extra_documented_top_level_dirs.is_empty() {
        message.push_str(
            "- remove every documented top-level `src/*/` directory section that no longer exists in the real source tree\n",
        );
    }
    if documented_top_level_dirs != expected_top_level_dirs {
        message.push_str(
            "- reorder every documented top-level `src/*/` directory entry by alphabetical order\n",
        );
        message.push_str(
            "- keep the explanations, but do not keep a semantically grouped order that violates the alphabetic rule\n",
        );
    }
    if !missing_dependency_policy_modules.is_empty() {
        message.push_str(
            "- add every new top-level module to the explicit dependency policy instead of letting it bypass architecture checks\n",
        );
    }
    if !stale_dependency_policy_modules.is_empty() {
        message.push_str(
            "- remove dependency-policy entries for top-level modules that no longer exist after refactors or renames\n",
        );
    }
    if !unknown_policy_dependencies.is_empty() {
        message.push_str(
            "- only reference real policy-controlled top-level modules in dependency allow-lists; fix typos and stale names\n",
        );
    }
    if !dependency_violations.is_empty() {
        message.push_str(
            "- move the shared logic into a lower layer or shared model module instead of reaching upward across architecture boundaries\n",
        );
        message.push_str(
            "- only widen the dependency policy after the boundary is explicitly redefined in the architecture docs\n",
        );
    }
    message.push_str(
        "\nWhy this blocks the change:\n\
- `source-layout.md` is the architectural index for real source boundaries; silent drift and ad hoc ordering are not allowed\n\
- the dependency policy is itself part of the architecture contract, so new top-level modules must be declared explicitly\n",
    );
    message
}

fn render_bytes(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "(empty)".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_path(path: &Path, repo_root: &Path) -> String {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str())
        .filter(|component| !component.is_empty())
        .map(os_str_to_string)
        .collect::<Vec<_>>()
        .join("/")
}

fn os_str_to_string(value: &OsStr) -> String {
    value.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        SourceSnapshot, collect_actual_architecture_modules, collect_actual_top_level_src_dirs,
        collect_dependency_violations, collect_documented_source_files,
        collect_documented_top_level_src_dirs, collect_unknown_policy_dependencies,
        extract_crate_module_dependencies, format_validation_failure,
    };

    #[test]
    fn parses_documented_files_from_nested_source_sections() {
        let doc = "\
- `src/artifact/`
  - `install_source.rs`：...
- `src/application/`
  - `bootstrap_use_case.rs`：...
";
        let files = collect_documented_source_files(doc);
        assert!(files.contains(&"src/artifact/install_source.rs".to_string()));
        assert!(files.contains(&"src/application/bootstrap_use_case.rs".to_string()));
    }

    #[test]
    fn collects_top_level_src_dirs_in_document_order() {
        let doc = "\
- `src/application/`
- `src/artifact/`
- `src/install_plan/`
";
        let dirs = collect_documented_top_level_src_dirs(doc);
        assert_eq!(dirs, vec!["src/application", "src/artifact", "src/install_plan"]);
    }

    #[test]
    fn collects_actual_top_level_src_dirs_from_source_files() {
        let dirs = collect_actual_top_level_src_dirs(&[
            "src/application/mod.rs".to_string(),
            "src/install_plan/mod.rs".to_string(),
            "src/download_sources.rs".to_string(),
            "src/artifact/mod.rs".to_string(),
        ]);
        assert_eq!(dirs, vec!["src/application", "src/artifact", "src/install_plan"]);
    }

    #[test]
    fn renders_combined_failure_message() {
        let message = format_validation_failure(
            "staged index",
            &["src/install_plan/mod.rs".to_string()],
            &["src/legacy.rs".to_string()],
            &["src/contracts".to_string()],
            &["src/platform".to_string()],
            &["src/application".to_string(), "src/artifact".to_string()],
            &["src/artifact".to_string(), "src/application".to_string()],
            &["future_domain".to_string()],
            &["legacy_domain".to_string()],
            &["typo_domain".to_string()],
            &[],
        );
        assert!(message.contains("Missing file entries:"));
        assert!(message.contains("Extra documented file entries:"));
        assert!(message.contains("Missing top-level src directory entries:"));
        assert!(message.contains("Extra documented top-level src directory entries:"));
        assert!(message.contains("Documented top-level src directory order:"));
        assert!(message.contains("Expected alphabetical order:"));
        assert!(message.contains("Top-level modules missing dependency policy entries:"));
        assert!(message.contains("Stale dependency policy modules:"));
        assert!(message.contains("Unknown dependency policy targets:"));
    }

    #[test]
    fn extracts_known_crate_dependencies_from_source_text() {
        let dependencies = extract_crate_module_dependencies(
            "use crate::contracts::BootstrapItem;\nlet _ = crate::managed_toolchain::x();",
            &super::known_architecture_modules(),
        );
        assert!(dependencies.contains("contracts"));
        assert!(dependencies.contains("managed_toolchain"));
    }

    #[test]
    fn flags_upward_dependency_direction_violation() {
        let violations = collect_dependency_violations(&[
            SourceSnapshot {
                path: "src/managed_toolchain/mod.rs".to_string(),
                contents: "use crate::install_plan::resolved_plan_item::ResolvedPlanItem;"
                    .to_string(),
            },
            SourceSnapshot {
                path: "src/install_plan/mod.rs".to_string(),
                contents: String::new(),
            },
        ]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].source_module, "managed_toolchain");
        assert_eq!(violations[0].referenced_module, "install_plan");
    }

    #[test]
    fn collects_actual_architecture_modules_from_top_level_paths() {
        let modules = collect_actual_architecture_modules(&[
            "src/application/mod.rs".to_string(),
            "src/future_domain/mod.rs".to_string(),
            "src/download_sources.rs".to_string(),
            "src/main.rs".to_string(),
            "src/cli.rs".to_string(),
        ]);
        assert!(modules.contains("application"));
        assert!(modules.contains("future_domain"));
        assert!(modules.contains("download_sources"));
        assert!(!modules.contains("main"));
        assert!(!modules.contains("cli"));
    }

    #[test]
    fn policy_targets_must_exist_in_known_module_set() {
        let unknown_dependencies =
            collect_unknown_policy_dependencies(&super::known_architecture_modules());
        assert!(unknown_dependencies.is_empty());
    }
}
