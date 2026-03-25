#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

GH_RELEASE_REPO = "cli/cli"
GH_BOOTSTRAP_PHASE = "bootstrap_gh"
UV_BOOTSTRAP_PHASE = "bootstrap_uv"
GH_RELEASE_PHASE = "release_gh"
ARCHIVE_TREE_RELEASE_PHASE = "archive_tree_release"
GIT_BOOTSTRAP_PHASE = "bootstrap_git"
PIP_PHASE = "pip"
SYSTEM_PACKAGE_PHASE = "system_package"
APT_PHASE = "apt"
UV_PHASE = "uv"
UV_PYTHON_PHASE = "uv_python"
UV_TOOL_PHASE = "uv_tool"
NPM_GLOBAL_PHASE = "npm_global"
WORKSPACE_PACKAGE_PHASE = "workspace_package"
CARGO_INSTALL_PHASE = "cargo_install"
RUSTUP_COMPONENT_PHASE = "rustup_component"
GO_INSTALL_PHASE = "go_install"
DOWNLOAD_ATTEMPTS = 5
HOST_INSTALL_ATTEMPTS = 3

NODE_ARCHIVE_VERSION = "v22.14.0"
GO_ARCHIVE_VERSION = "1.23.7"
TEMURIN_JDK_MAJOR = "21"
PIP_PACKAGE = "boltons==24.0.0"
PIP_IMPORT = "boltons"
UV_PYTHON_VERSION = "3.13.12"
NPM_GLOBAL_PACKAGE = "http-server@14.1.1"
NPM_GLOBAL_BINARY = "http-server"
GO_INSTALL_BINARY = "hello"
CARGO_FIXTURE_BINARY = "ti-cargo-fixture"


class SmokeError(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run live install smoke tests for toolchain-installer on the current host."
    )
    parser.add_argument(
        "--binary",
        default="target/release/toolchain-installer",
        help="Path to the built toolchain-installer binary.",
    )
    parser.add_argument(
        "--phase",
        action="append",
        dest="phases",
        choices=sorted(
            {
                GH_BOOTSTRAP_PHASE,
                UV_BOOTSTRAP_PHASE,
                GH_RELEASE_PHASE,
                ARCHIVE_TREE_RELEASE_PHASE,
                GIT_BOOTSTRAP_PHASE,
                PIP_PHASE,
                SYSTEM_PACKAGE_PHASE,
                APT_PHASE,
                UV_PHASE,
                UV_PYTHON_PHASE,
                UV_TOOL_PHASE,
                NPM_GLOBAL_PHASE,
                WORKSPACE_PACKAGE_PHASE,
                CARGO_INSTALL_PHASE,
                RUSTUP_COMPONENT_PHASE,
                GO_INSTALL_PHASE,
            }
        ),
        help="Run only the selected phase. May be passed multiple times.",
    )
    parser.add_argument(
        "--keep-temp",
        action="store_true",
        help="Keep the temporary workspace on success for inspection.",
    )
    return parser.parse_args()


def resolve_binary(raw_path: str) -> Path:
    candidate = Path(raw_path)
    if candidate.exists():
        return candidate.resolve()
    if os.name == "nt" and candidate.suffix.lower() != ".exe":
        windows_candidate = candidate.with_suffix(".exe")
        if windows_candidate.exists():
            return windows_candidate.resolve()
    raise SmokeError(f"toolchain-installer binary not found: {candidate}")


def run_command(
    argv: list[str | Path],
    *,
    env: dict[str, str] | None = None,
    expect_success: bool = True,
    cwd: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    printable = " ".join(str(part) for part in argv)
    print(f"$ {printable}", flush=True)
    completed = subprocess.run(
        [str(part) for part in argv],
        env=env,
        cwd=str(cwd) if cwd else None,
        text=True,
        capture_output=True,
        check=False,
    )
    if expect_success and completed.returncode != 0:
        raise SmokeError(
            f"command failed with exit {completed.returncode}: {printable}\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}"
        )
    return completed


def run_installer_json(
    binary: Path,
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    attempts: int = 1,
) -> dict:
    last_error: Exception | None = None
    for attempt in range(1, attempts + 1):
        try:
            completed = run_command([binary, *args], env=env)
            try:
                return json.loads(completed.stdout)
            except json.JSONDecodeError as err:
                raise SmokeError(
                    f"installer output is not valid json for args {args}: {err}\n"
                    f"stdout:\n{completed.stdout}\n"
                    f"stderr:\n{completed.stderr}"
                ) from err
        except SmokeError as err:
            last_error = err
            if attempt == attempts:
                break
            print(
                f"installer attempt {attempt}/{attempts} failed; retrying in {attempt}s",
                file=sys.stderr,
                flush=True,
            )
            time.sleep(attempt)
    assert last_error is not None
    raise last_error


def fetch_json(
    url: str,
    *,
    attempts: int = DOWNLOAD_ATTEMPTS,
    accept: str = "application/json",
) -> object:
    last_error: Exception | None = None
    github_token = os.environ.get("GITHUB_TOKEN") or os.environ.get(
        "TOOLCHAIN_INSTALLER_GITHUB_TOKEN"
    )
    for attempt in range(1, attempts + 1):
        headers = {
            "Accept": accept,
            "User-Agent": "toolchain-installer-install-smoke",
        }
        if github_token:
            headers["Authorization"] = f"Bearer {github_token}"
        request = urllib.request.Request(
            url,
            headers=headers,
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except Exception as err:
            last_error = err
            if attempt == attempts:
                break
            print(
                f"http fetch attempt {attempt}/{attempts} failed for {url}; retrying in {attempt}s",
                file=sys.stderr,
                flush=True,
            )
            time.sleep(attempt)
    assert last_error is not None
    raise SmokeError(f"failed to fetch json from {url}: {last_error}") from last_error


def single_item(result: dict) -> dict:
    items = result.get("items")
    if not isinstance(items, list) or len(items) != 1:
        raise SmokeError(f"expected exactly one result item, got: {result}")
    return items[0]


def require_installed(item: dict, *, phase: str) -> Path:
    status = item.get("status")
    if status != "installed":
        raise SmokeError(f"{phase} expected installed status, got {status}: {item}")
    destination = item.get("destination")
    if not destination:
        raise SmokeError(f"{phase} expected destination in result: {item}")
    destination_path = Path(destination)
    if not destination_path.exists():
        raise SmokeError(f"{phase} destination does not exist: {destination_path}")
    return destination_path


def executable_suffix(target_triple: str) -> str:
    return ".exe" if "windows" in target_triple else ""


def gh_asset_suffix_for_target(target_triple: str) -> str:
    mapping = {
        "x86_64-unknown-linux-gnu": "_linux_amd64.tar.gz",
        "aarch64-unknown-linux-gnu": "_linux_arm64.tar.gz",
        "x86_64-apple-darwin": "_macOS_amd64.zip",
        "aarch64-apple-darwin": "_macOS_arm64.zip",
        "x86_64-pc-windows-msvc": "_windows_amd64.zip",
        "aarch64-pc-windows-msvc": "_windows_arm64.zip",
    }
    try:
        return mapping[target_triple]
    except KeyError as err:
        raise SmokeError(f"unsupported target triple for gh release smoke: {target_triple}") from err


def find_asset_for_suffix(release: dict, suffix: str) -> dict:
    for asset in release.get("assets", []):
        if asset.get("name", "").endswith(suffix):
            return asset
    raise SmokeError(f"cannot find release asset with suffix {suffix!r}")


def masked_path_env() -> dict[str, str]:
    env = os.environ.copy()
    hidden_dir = tempfile.mkdtemp(prefix="ti-hidden-path-")
    env["PATH"] = hidden_dir
    return env


def filtered_path_env(*, hidden_commands: set[str]) -> dict[str, str]:
    if os.name == "nt":
        return masked_path_env()

    env = os.environ.copy()
    visible_dir = Path(tempfile.mkdtemp(prefix="ti-filtered-path-"))
    hidden = {value.lower() for value in hidden_commands}

    for raw_dir in os.environ.get("PATH", "").split(os.pathsep):
        if not raw_dir:
            continue
        path_dir = Path(raw_dir)
        if not path_dir.is_dir():
            continue
        for entry in path_dir.iterdir():
            try:
                is_executable = (
                    (entry.is_file() or entry.is_symlink()) and os.access(entry, os.X_OK)
                )
            except OSError:
                continue
            if not is_executable or entry.name.lower() in hidden:
                continue
            link = visible_dir / entry.name
            if link.exists():
                continue
            try:
                os.symlink(entry, link)
            except OSError:
                continue

    env["PATH"] = str(visible_dir)
    return env


def find_relative_path_under(root: Path, relative_path: str) -> Path:
    relative = Path(relative_path)
    direct_candidate = root / relative
    if direct_candidate.exists():
        return direct_candidate

    needle = relative.parts[-1]
    suffix_parts = relative.parts
    for candidate in root.rglob(needle):
        try:
            rel = candidate.relative_to(root)
        except ValueError:
            continue
        if len(rel.parts) >= len(suffix_parts) and rel.parts[-len(suffix_parts) :] == suffix_parts:
            return candidate

    raise SmokeError(f"cannot locate `{relative_path}` under extracted root {root}")


def relative_root(match_path: Path, relative_path: str) -> Path:
    root = match_path
    for _ in Path(relative_path).parts:
        root = root.parent
    return root


def package_json_path(workspace_dir: Path, package_name: str) -> Path:
    package_dir = workspace_dir / "node_modules"
    for segment in package_name.split("/"):
        package_dir /= segment
    return package_dir / "package.json"


def platform_name(target_triple: str) -> str:
    if "windows" in target_triple:
        return "windows"
    if "apple-darwin" in target_triple:
        return "macos"
    if "linux" in target_triple:
        return "linux"
    raise SmokeError(f"unsupported target triple: {target_triple}")


def default_phases_for_target(target_triple: str) -> list[str]:
    phases = [
        GH_BOOTSTRAP_PHASE,
        UV_BOOTSTRAP_PHASE,
        GH_RELEASE_PHASE,
        ARCHIVE_TREE_RELEASE_PHASE,
        PIP_PHASE,
        UV_PHASE,
        UV_PYTHON_PHASE,
        UV_TOOL_PHASE,
        NPM_GLOBAL_PHASE,
        WORKSPACE_PACKAGE_PHASE,
        CARGO_INSTALL_PHASE,
        RUSTUP_COMPONENT_PHASE,
        GO_INSTALL_PHASE,
    ]
    platform_id = platform_name(target_triple)
    phases.append(GIT_BOOTSTRAP_PHASE)
    if platform_id != "windows":
        phases.append(SYSTEM_PACKAGE_PHASE)
        if platform_id == "linux":
            phases.append(APT_PHASE)
    return phases


def node_archive_filename(target_triple: str) -> str:
    mapping = {
        "x86_64-unknown-linux-gnu": f"node-{NODE_ARCHIVE_VERSION}-linux-x64.tar.xz",
        "aarch64-unknown-linux-gnu": f"node-{NODE_ARCHIVE_VERSION}-linux-arm64.tar.xz",
        "x86_64-apple-darwin": f"node-{NODE_ARCHIVE_VERSION}-darwin-x64.tar.gz",
        "aarch64-apple-darwin": f"node-{NODE_ARCHIVE_VERSION}-darwin-arm64.tar.gz",
        "x86_64-pc-windows-msvc": f"node-{NODE_ARCHIVE_VERSION}-win-x64.zip",
        "aarch64-pc-windows-msvc": f"node-{NODE_ARCHIVE_VERSION}-win-arm64.zip",
    }
    try:
        return mapping[target_triple]
    except KeyError as err:
        raise SmokeError(f"unsupported target triple for node archive smoke: {target_triple}") from err


def go_archive_filename(target_triple: str) -> str:
    mapping = {
        "x86_64-unknown-linux-gnu": f"go{GO_ARCHIVE_VERSION}.linux-amd64.tar.gz",
        "aarch64-unknown-linux-gnu": f"go{GO_ARCHIVE_VERSION}.linux-arm64.tar.gz",
        "x86_64-apple-darwin": f"go{GO_ARCHIVE_VERSION}.darwin-amd64.tar.gz",
        "aarch64-apple-darwin": f"go{GO_ARCHIVE_VERSION}.darwin-arm64.tar.gz",
        "x86_64-pc-windows-msvc": f"go{GO_ARCHIVE_VERSION}.windows-amd64.zip",
        "aarch64-pc-windows-msvc": f"go{GO_ARCHIVE_VERSION}.windows-arm64.zip",
    }
    try:
        return mapping[target_triple]
    except KeyError as err:
        raise SmokeError(f"unsupported target triple for go archive smoke: {target_triple}") from err


def temurin_api_target(target_triple: str) -> tuple[str, str]:
    mapping = {
        "x86_64-unknown-linux-gnu": ("linux", "x64"),
        "aarch64-unknown-linux-gnu": ("linux", "aarch64"),
        "x86_64-apple-darwin": ("mac", "x64"),
        "aarch64-apple-darwin": ("mac", "aarch64"),
        "x86_64-pc-windows-msvc": ("windows", "x64"),
        "aarch64-pc-windows-msvc": ("windows", "aarch64"),
    }
    try:
        return mapping[target_triple]
    except KeyError as err:
        raise SmokeError(f"unsupported target triple for temurin archive smoke: {target_triple}") from err


def verify_version_contains(binary: Path, *args: str, expected_fragment: str) -> None:
    completed = run_command([binary, *args])
    combined = f"{completed.stdout}\n{completed.stderr}"
    if expected_fragment not in combined:
        raise SmokeError(
            f"expected {binary} output to contain {expected_fragment!r}, got:\n{combined}"
        )


def phase_bootstrap_gh(binary: Path, target_triple: str, workspace: Path) -> None:
    managed_dir = workspace / "bootstrap-gh"
    result = run_installer_json(
        binary,
        ["bootstrap", "--json", "--managed-dir", str(managed_dir), "--tool", "gh"],
        env=masked_path_env(),
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    destination = require_installed(item, phase=GH_BOOTSTRAP_PHASE)
    verify_version_contains(destination, "--version", expected_fragment="gh version")
    print(f"{GH_BOOTSTRAP_PHASE}: ok -> {destination}", flush=True)


def phase_bootstrap_uv(binary: Path, target_triple: str, workspace: Path) -> None:
    managed_dir = workspace / "bootstrap-uv"
    result = run_installer_json(
        binary,
        ["bootstrap", "--json", "--managed-dir", str(managed_dir), "--tool", "uv"],
        env=masked_path_env(),
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    destination = require_installed(item, phase=UV_BOOTSTRAP_PHASE)
    verify_version_contains(destination, "--version", expected_fragment="uv ")
    print(f"{UV_BOOTSTRAP_PHASE}: ok -> {destination}", flush=True)


def phase_release_gh(binary: Path, target_triple: str, workspace: Path) -> None:
    managed_dir = workspace / "release-managed"
    destination = workspace / f"gh{executable_suffix(target_triple)}"
    release = fetch_json(f"https://api.github.com/repos/{GH_RELEASE_REPO}/releases/latest")
    asset = find_asset_for_suffix(release, gh_asset_suffix_for_target(target_triple))
    args = [
        "--json",
        "--managed-dir",
        str(managed_dir),
        "--method",
        "release",
        "--id",
        "gh-release",
        "--url",
        asset["browser_download_url"],
        "--binary-name",
        f"gh{executable_suffix(target_triple)}",
        "--destination",
        str(destination),
    ]
    if "windows" in target_triple:
        args.extend(["--archive-binary", f"bin/gh{executable_suffix(target_triple)}"])
    digest = asset.get("digest")
    if isinstance(digest, str) and digest.strip():
        args.extend(["--sha256", digest.split(":", 1)[-1]])
    result = run_installer_json(binary, args, attempts=DOWNLOAD_ATTEMPTS)
    item = single_item(result)
    installed = require_installed(item, phase=GH_RELEASE_PHASE)
    verify_version_contains(installed, "--version", expected_fragment="gh version")
    print(f"{GH_RELEASE_PHASE}: ok -> {installed}", flush=True)


def strip_archive_suffix(asset_name: str) -> str:
    for suffix in (".tar.gz", ".tar.xz", ".zip"):
        if asset_name.endswith(suffix):
            return asset_name[: -len(suffix)]
    raise SmokeError(f"unsupported archive suffix: {asset_name}")


def phase_archive_tree_release(binary: Path, target_triple: str, workspace: Path) -> None:
    managed_dir = workspace / "archive-tree-managed"
    release = fetch_json(
        f"https://api.github.com/repos/{GH_RELEASE_REPO}/releases/latest",
        accept="application/vnd.github+json",
    )
    if not isinstance(release, dict):
        raise SmokeError(f"unexpected GitHub release payload: {release!r}")
    asset = find_asset_for_suffix(release, gh_asset_suffix_for_target(target_triple))

    temurin_os, temurin_arch = temurin_api_target(target_triple)
    temurin_assets = fetch_json(
        "https://api.adoptium.net/v3/assets/latest/"
        f"{TEMURIN_JDK_MAJOR}/hotspot?architecture={temurin_arch}"
        f"&image_type=jdk&os={temurin_os}&vendor=eclipse"
    )
    if not isinstance(temurin_assets, list) or not temurin_assets:
        raise SmokeError(f"unexpected temurin payload: {temurin_assets!r}")
    temurin_asset = temurin_assets[0]
    temurin_package = temurin_asset["binary"]["package"]

    node_filename = node_archive_filename(target_triple)
    go_filename = go_archive_filename(target_triple)
    ext = executable_suffix(target_triple)
    archive_specs = [
        {
            "id": "gh-tree",
            "url": asset["browser_download_url"],
            "sha256": asset.get("digest", "").split(":", 1)[-1] if asset.get("digest") else "",
            "binary_relative": f"bin/gh{ext}",
            "support_relative": "LICENSE",
            "version_args": ["--version"],
            "expected_fragment": "gh version",
        },
        {
            "id": "node-tree",
            "url": f"https://nodejs.org/dist/{NODE_ARCHIVE_VERSION}/{node_filename}",
            "sha256": "",
            "binary_relative": f"node{ext}" if "windows" in target_triple else "bin/node",
            "support_relative": (
                "node_modules/npm/package.json"
                if "windows" in target_triple
                else "lib/node_modules/npm/package.json"
            ),
            "version_args": ["--version"],
            "expected_fragment": NODE_ARCHIVE_VERSION,
        },
        {
            "id": "go-sdk-tree",
            "url": f"https://dl.google.com/go/{go_filename}",
            "sha256": "",
            "binary_relative": f"go/bin/go{ext}",
            "support_relative": "go/src/go.mod",
            "version_args": ["version"],
            "expected_fragment": f"go{GO_ARCHIVE_VERSION}",
        },
        {
            "id": "temurin-jdk-tree",
            "url": temurin_package["link"],
            "sha256": temurin_package["checksum"],
            "binary_relative": f"bin/java{ext}",
            "support_relative": "lib/modules",
            "version_args": ["-version"],
            "expected_fragment": temurin_asset["version"]["openjdk_version"].split("+", 1)[0],
        },
    ]

    for spec in archive_specs:
        destination = workspace / spec["id"]
        args = [
            "--json",
            "--managed-dir",
            str(managed_dir),
            "--method",
            ARCHIVE_TREE_RELEASE_PHASE,
            "--id",
            spec["id"],
            "--url",
            spec["url"],
            "--destination",
            str(destination),
        ]
        if spec["sha256"]:
            args.extend(["--sha256", spec["sha256"]])
        result = run_installer_json(binary, args, attempts=DOWNLOAD_ATTEMPTS)
        item = single_item(result)
        extracted_root = require_installed(item, phase=ARCHIVE_TREE_RELEASE_PHASE)
        installed_binary = find_relative_path_under(extracted_root, spec["binary_relative"])
        find_relative_path_under(extracted_root, spec["support_relative"])
        verify_version_contains(
            installed_binary,
            *spec["version_args"],
            expected_fragment=spec["expected_fragment"],
        )
        root_dir = relative_root(installed_binary, spec["binary_relative"])
        print(f"{ARCHIVE_TREE_RELEASE_PHASE}:{spec['id']}: ok -> {root_dir}", flush=True)


def phase_bootstrap_git(binary: Path, target_triple: str, workspace: Path) -> None:
    managed_dir = workspace / "bootstrap-git"
    env = masked_path_env() if "windows" in target_triple else filtered_path_env(hidden_commands={"git"})
    result = run_installer_json(
        binary,
        ["bootstrap", "--json", "--managed-dir", str(managed_dir), "--tool", "git"],
        env=env,
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    if item.get("status") != "installed":
        raise SmokeError(f"{GIT_BOOTSTRAP_PHASE} expected installed status, got: {item}")
    if "windows" in target_triple:
        destination = require_installed(item, phase=GIT_BOOTSTRAP_PHASE)
        verify_version_contains(destination, "--version", expected_fragment="git version")
        print(f"{GIT_BOOTSTRAP_PHASE}: ok -> {destination}", flush=True)
        return

    source = item.get("source")
    if not isinstance(source, str) or not source.startswith("system:"):
        raise SmokeError(f"{GIT_BOOTSTRAP_PHASE} expected system package source, got: {item}")
    verify_version_contains(Path(shutil.which("git") or "git"), "--version", expected_fragment="git version")
    print(f"{GIT_BOOTSTRAP_PHASE}: ok -> {source}", flush=True)


def phase_pip(binary: Path) -> None:
    result = run_installer_json(
        binary,
        [
            "--json",
            "--method",
            "pip",
            "--id",
            "boltons",
            "--package",
            PIP_PACKAGE,
            "--python",
            sys.executable,
        ],
        attempts=HOST_INSTALL_ATTEMPTS,
    )
    item = single_item(result)
    if item.get("status") != "installed":
        raise SmokeError(f"{PIP_PHASE} expected installed status, got: {item}")
    run_command(
        [
            sys.executable,
            "-c",
            f"import {PIP_IMPORT}; print({PIP_IMPORT}.__file__)",
        ]
    )
    print(f"{PIP_PHASE}: ok", flush=True)


def phase_system_package(binary: Path) -> None:
    result = run_installer_json(
        binary,
        [
            "--json",
            "--method",
            "system_package",
            "--id",
            "jq-system-package",
            "--package",
            "jq",
        ],
        attempts=HOST_INSTALL_ATTEMPTS,
    )
    item = single_item(result)
    if item.get("status") != "installed":
        raise SmokeError(f"{SYSTEM_PACKAGE_PHASE} expected installed status, got: {item}")
    run_command(["jq", "--version"])
    print(f"{SYSTEM_PACKAGE_PHASE}: ok", flush=True)


def phase_apt(binary: Path) -> None:
    result = run_installer_json(
        binary,
        [
            "--json",
            "--method",
            "apt",
            "--id",
            "jq-apt",
            "--package",
            "jq",
        ],
        attempts=HOST_INSTALL_ATTEMPTS,
    )
    item = single_item(result)
    if item.get("status") != "installed":
        raise SmokeError(f"{APT_PHASE} expected installed status, got: {item}")
    run_command(["jq", "--version"])
    print(f"{APT_PHASE}: ok", flush=True)


def phase_uv(binary: Path, managed_dir: Path) -> None:
    result = run_installer_json(
        binary,
        ["--json", "--managed-dir", str(managed_dir), "--method", "uv", "--id", "uv"],
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    destination = require_installed(item, phase=UV_PHASE)
    verify_version_contains(destination, "--version", expected_fragment="uv ")
    print(f"{UV_PHASE}: ok -> {destination}", flush=True)


def resolve_python_destination(destination: Path, target_triple: str) -> Path:
    if destination.is_file():
        return destination
    ext = executable_suffix(target_triple)
    preferred_names = [f"python3.13{ext}", f"python3{ext}", f"python{ext}"]
    for name in preferred_names:
        candidate = destination / name
        if candidate.exists():
            return candidate
    for candidate in destination.rglob(f"python*{ext}"):
        if candidate.is_file():
            return candidate
    raise SmokeError(f"cannot locate installed python under {destination}")


def phase_uv_python(binary: Path, target_triple: str, managed_dir: Path) -> None:
    result = run_installer_json(
        binary,
        [
            "--json",
            "--managed-dir",
            str(managed_dir),
            "--method",
            UV_PYTHON_PHASE,
            "--id",
            f"python{UV_PYTHON_VERSION}",
            "--tool-version",
            UV_PYTHON_VERSION,
        ],
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    destination = require_installed(item, phase=UV_PYTHON_PHASE)
    python_binary = resolve_python_destination(destination, target_triple)
    verify_version_contains(python_binary, "--version", expected_fragment=UV_PYTHON_VERSION)
    print(f"{UV_PYTHON_PHASE}: ok -> {python_binary}", flush=True)


def phase_uv_tool(binary: Path, target_triple: str, managed_dir: Path) -> None:
    tool_specs = [
        {"id": "ruff", "package": "ruff", "expected_fragment": "ruff "},
        {"id": "mypy", "package": "mypy", "expected_fragment": "mypy "},
        {"id": "pytest", "package": "pytest", "expected_fragment": "pytest "},
    ]
    for spec in tool_specs:
        result = run_installer_json(
            binary,
            [
                "--json",
                "--managed-dir",
                str(managed_dir),
                "--method",
                UV_TOOL_PHASE,
                "--id",
                spec["id"],
                "--package",
                spec["package"],
                "--python",
                UV_PYTHON_VERSION,
            ],
            attempts=DOWNLOAD_ATTEMPTS,
        )
        item = single_item(result)
        destination = require_installed(item, phase=UV_TOOL_PHASE)
        verify_version_contains(destination, "--version", expected_fragment=spec["expected_fragment"])
        print(f"{UV_TOOL_PHASE}:{spec['id']}: ok -> {destination}", flush=True)


def phase_npm_global(binary: Path, target_triple: str, workspace: Path) -> None:
    manager_specs = ["npm", "pnpm", "bun"]
    for manager in manager_specs:
        managed_dir = workspace / f"npm-global-{manager}"
        result = run_installer_json(
            binary,
            [
                "--json",
                "--managed-dir",
                str(managed_dir),
                "--method",
                NPM_GLOBAL_PHASE,
                "--id",
                f"{NPM_GLOBAL_BINARY}-{manager}",
                "--package",
                NPM_GLOBAL_PACKAGE,
                "--binary-name",
                NPM_GLOBAL_BINARY,
                "--manager",
                manager,
            ],
            attempts=HOST_INSTALL_ATTEMPTS,
        )
        item = single_item(result)
        destination = require_installed(item, phase=NPM_GLOBAL_PHASE)
        verify_version_contains(destination, "--version", expected_fragment="14.1.1")
        print(f"{NPM_GLOBAL_PHASE}:{manager}: ok -> {destination}", flush=True)


def phase_workspace_package(binary: Path, workspace: Path) -> None:
    package_specs = [
        {"id": "react", "package": "react@19.2.4", "package_name": "react", "manager": "npm"},
        {"id": "nextjs", "package": "next@16.2.1", "package_name": "next", "manager": "npm"},
        {"id": "vite", "package": "vite@8.0.2", "package_name": "vite", "manager": "npm"},
        {
            "id": "react-router",
            "package": "react-router@7.13.2",
            "package_name": "react-router",
            "manager": "npm",
        },
        {
            "id": "heroui",
            "package": "@heroui/react@3.0.1",
            "package_name": "@heroui/react",
            "manager": "pnpm",
        },
        {
            "id": "spectrum",
            "package": "@adobe/react-spectrum@3.46.2",
            "package_name": "@adobe/react-spectrum",
            "manager": "pnpm",
        },
        {
            "id": "shadcn",
            "package": "shadcn@4.1.0",
            "package_name": "shadcn",
            "manager": "bun",
        },
    ]

    for spec in package_specs:
        workspace_dir = workspace / f"workspace-package-{spec['id']}"
        workspace_dir.mkdir(parents=True, exist_ok=True)
        (workspace_dir / "package.json").write_text(
            json.dumps({"name": f"ti-{spec['id']}", "private": True}, indent=2),
            encoding="utf-8",
        )
        result = run_installer_json(
            binary,
            [
                "--json",
                "--method",
                WORKSPACE_PACKAGE_PHASE,
                "--id",
                spec["id"],
                "--package",
                spec["package"],
                "--destination",
                str(workspace_dir),
                "--manager",
                spec["manager"],
            ],
            attempts=HOST_INSTALL_ATTEMPTS,
        )
        item = single_item(result)
        installed_dir = require_installed(item, phase=WORKSPACE_PACKAGE_PHASE)
        installed_package_json = package_json_path(installed_dir, spec["package_name"])
        if not installed_package_json.exists():
            raise SmokeError(
                f"{WORKSPACE_PACKAGE_PHASE} missing installed package metadata: {installed_package_json}"
            )
        print(f"{WORKSPACE_PACKAGE_PHASE}:{spec['id']}: ok -> {installed_package_json}", flush=True)


def cargo_fixture_dir() -> Path:
    return Path(__file__).resolve().parent.parent / "fixtures" / "cargo-install-fixture"


def go_fixture_dir() -> Path:
    return (
        Path(__file__).resolve().parent.parent
        / "fixtures"
        / "go-install-fixture"
        / "cmd"
        / GO_INSTALL_BINARY
    )


def phase_cargo_install(binary: Path, workspace: Path) -> None:
    managed_dir = workspace / "cargo-install-managed"
    cargo_specs = [
        {
            "id": CARGO_FIXTURE_BINARY,
            "package": str(cargo_fixture_dir()),
            "binary_name": CARGO_FIXTURE_BINARY,
            "expected_fragment": "0.1.0",
        },
        {
            "id": "cargo-nextest",
            "package": "cargo-nextest",
            "binary_name": "cargo-nextest",
            "version": "0.9.132",
            "expected_fragment": "cargo-nextest 0.9.132",
        },
    ]
    for spec in cargo_specs:
        args = [
            "--json",
            "--managed-dir",
            str(managed_dir),
            "--method",
            CARGO_INSTALL_PHASE,
            "--id",
            spec["id"],
            "--package",
            spec["package"],
            "--binary-name",
            spec["binary_name"],
        ]
        version = spec.get("version")
        if version:
            args.extend(["--tool-version", version])
        result = run_installer_json(binary, args, attempts=HOST_INSTALL_ATTEMPTS)
        item = single_item(result)
        destination = require_installed(item, phase=CARGO_INSTALL_PHASE)
        verify_version_contains(destination, "--version", expected_fragment=spec["expected_fragment"])
        print(f"{CARGO_INSTALL_PHASE}:{spec['id']}: ok -> {destination}", flush=True)


def phase_rustup_component(binary: Path) -> None:
    component_specs = [
        {
            "id": "rustfmt",
            "package": "rustfmt",
            "binary_name": "rustfmt",
            "expected_fragment": "rustfmt",
        },
        {
            "id": "clippy",
            "package": "clippy",
            "binary_name": "cargo-clippy",
            "expected_fragment": "clippy",
        },
    ]
    for spec in component_specs:
        result = run_installer_json(
            binary,
            [
                "--json",
                "--method",
                RUSTUP_COMPONENT_PHASE,
                "--id",
                spec["id"],
                "--package",
                spec["package"],
                "--binary-name",
                spec["binary_name"],
            ],
            attempts=HOST_INSTALL_ATTEMPTS,
        )
        item = single_item(result)
        destination = require_installed(item, phase=RUSTUP_COMPONENT_PHASE)
        verify_version_contains(destination, "--version", expected_fragment=spec["expected_fragment"])
        print(f"{RUSTUP_COMPONENT_PHASE}:{spec['id']}: ok -> {destination}", flush=True)


def phase_go_install(binary: Path, workspace: Path) -> None:
    managed_dir = workspace / "go-install-managed"
    go_specs = [
        {
            "id": GO_INSTALL_BINARY,
            "package": str(go_fixture_dir()),
            "binary_name": GO_INSTALL_BINARY,
            "version_args": [],
            "expected_fragment": "Hello, world!",
        },
        {
            "id": "gopls",
            "package": "golang.org/x/tools/gopls",
            "binary_name": "gopls",
            "version_args": ["version"],
            "expected_fragment": "gopls",
        },
        {
            "id": "golangci-lint",
            "package": "github.com/golangci/golangci-lint/cmd/golangci-lint",
            "binary_name": "golangci-lint",
            "version_args": ["--version"],
            "expected_fragment": "golangci-lint",
        },
    ]
    for spec in go_specs:
        result = run_installer_json(
            binary,
            [
                "--json",
                "--managed-dir",
                str(managed_dir),
                "--method",
                GO_INSTALL_PHASE,
                "--id",
                spec["id"],
                "--package",
                spec["package"],
                "--binary-name",
                spec["binary_name"],
            ],
            attempts=HOST_INSTALL_ATTEMPTS,
        )
        item = single_item(result)
        destination = require_installed(item, phase=GO_INSTALL_PHASE)
        if spec["version_args"]:
            verify_version_contains(
                destination,
                *spec["version_args"],
                expected_fragment=spec["expected_fragment"],
            )
        else:
            completed = run_command([destination], expect_success=True)
            combined = f"{completed.stdout}\n{completed.stderr}"
            if spec["expected_fragment"] not in combined:
                raise SmokeError(f"{GO_INSTALL_PHASE} unexpected output:\n{combined}")
        print(f"{GO_INSTALL_PHASE}:{spec['id']}: ok -> {destination}", flush=True)


def detect_target_triple(binary: Path) -> str:
    result = run_installer_json(binary, ["--json", "--method", "unknown", "--id", "probe"])
    target_triple = result.get("target_triple")
    if not isinstance(target_triple, str) or not target_triple:
        raise SmokeError(f"cannot determine target triple from installer output: {result}")
    return target_triple


def main() -> int:
    args = parse_args()
    binary = resolve_binary(args.binary)
    target_triple = detect_target_triple(binary)
    phases = args.phases or default_phases_for_target(target_triple)
    workspace = Path(tempfile.mkdtemp(prefix="ti-install-smoke-")).resolve()
    managed_dir = workspace / "managed"

    print(
        "install smoke configuration:",
        json.dumps(
            {
                "binary": str(binary),
                "target_triple": target_triple,
                "host_platform": {
                    "system": platform.system(),
                    "machine": platform.machine(),
                },
                "workspace": str(workspace),
                "phases": phases,
            },
            indent=2,
        ),
        sep="\n",
        flush=True,
    )

    try:
        for phase in phases:
            if phase == GH_BOOTSTRAP_PHASE:
                phase_bootstrap_gh(binary, target_triple, workspace)
            elif phase == UV_BOOTSTRAP_PHASE:
                phase_bootstrap_uv(binary, target_triple, workspace)
            elif phase == GH_RELEASE_PHASE:
                phase_release_gh(binary, target_triple, workspace)
            elif phase == ARCHIVE_TREE_RELEASE_PHASE:
                phase_archive_tree_release(binary, target_triple, workspace)
            elif phase == GIT_BOOTSTRAP_PHASE:
                phase_bootstrap_git(binary, target_triple, workspace)
            elif phase == PIP_PHASE:
                phase_pip(binary)
            elif phase == SYSTEM_PACKAGE_PHASE:
                phase_system_package(binary)
            elif phase == APT_PHASE:
                phase_apt(binary)
            elif phase == UV_PHASE:
                phase_uv(binary, managed_dir)
            elif phase == UV_PYTHON_PHASE:
                phase_uv_python(binary, target_triple, managed_dir)
            elif phase == UV_TOOL_PHASE:
                phase_uv_tool(binary, target_triple, managed_dir)
            elif phase == NPM_GLOBAL_PHASE:
                phase_npm_global(binary, target_triple, workspace)
            elif phase == WORKSPACE_PACKAGE_PHASE:
                phase_workspace_package(binary, workspace)
            elif phase == CARGO_INSTALL_PHASE:
                phase_cargo_install(binary, workspace)
            elif phase == RUSTUP_COMPONENT_PHASE:
                phase_rustup_component(binary)
            elif phase == GO_INSTALL_PHASE:
                phase_go_install(binary, workspace)
            else:
                raise SmokeError(f"unsupported phase: {phase}")
    except Exception:
        print(f"install smoke failed; workspace preserved at {workspace}", file=sys.stderr)
        raise
    else:
        if args.keep_temp:
            print(f"install smoke workspace kept at {workspace}", flush=True)
        else:
            shutil.rmtree(workspace)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SmokeError as err:
        print(err, file=sys.stderr)
        raise SystemExit(1)
