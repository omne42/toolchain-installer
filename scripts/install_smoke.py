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
GH_RELEASE_PHASE = "release_gh"
GIT_BOOTSTRAP_PHASE = "bootstrap_git"
PIP_PHASE = "pip"
SYSTEM_PACKAGE_PHASE = "system_package"
APT_PHASE = "apt"
UV_PHASE = "uv"
UV_PYTHON_PHASE = "uv_python"
UV_TOOL_PHASE = "uv_tool"
DOWNLOAD_ATTEMPTS = 5
HOST_INSTALL_ATTEMPTS = 3

PIP_PACKAGE = "boltons==24.0.0"
PIP_IMPORT = "boltons"
UV_PYTHON_VERSION = "3.13.12"
UV_TOOL_PACKAGE = "ruff"
UV_TOOL_ID = "ruff"


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
                GH_RELEASE_PHASE,
                GIT_BOOTSTRAP_PHASE,
                PIP_PHASE,
                SYSTEM_PACKAGE_PHASE,
                APT_PHASE,
                UV_PHASE,
                UV_PYTHON_PHASE,
                UV_TOOL_PHASE,
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


def fetch_json(url: str, *, attempts: int = DOWNLOAD_ATTEMPTS) -> dict:
    last_error: Exception | None = None
    for attempt in range(1, attempts + 1):
        request = urllib.request.Request(
            url,
            headers={
                "Accept": "application/vnd.github+json",
                "User-Agent": "toolchain-installer-install-smoke",
            },
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
        GH_RELEASE_PHASE,
        PIP_PHASE,
        UV_PHASE,
        UV_PYTHON_PHASE,
        UV_TOOL_PHASE,
    ]
    platform_id = platform_name(target_triple)
    if platform_id == "windows":
        phases.append(GIT_BOOTSTRAP_PHASE)
    else:
        phases.append(SYSTEM_PACKAGE_PHASE)
        if platform_id == "linux":
            phases.append(APT_PHASE)
    return phases


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
    digest = asset.get("digest")
    if isinstance(digest, str) and digest.strip():
        args.extend(["--sha256", digest.split(":", 1)[-1]])
    result = run_installer_json(binary, args, attempts=DOWNLOAD_ATTEMPTS)
    item = single_item(result)
    installed = require_installed(item, phase=GH_RELEASE_PHASE)
    verify_version_contains(installed, "--version", expected_fragment="gh version")
    print(f"{GH_RELEASE_PHASE}: ok -> {installed}", flush=True)


def phase_bootstrap_git(binary: Path, workspace: Path) -> None:
    managed_dir = workspace / "bootstrap-git"
    result = run_installer_json(
        binary,
        ["bootstrap", "--json", "--managed-dir", str(managed_dir), "--tool", "git"],
        env=masked_path_env(),
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    destination = require_installed(item, phase=GIT_BOOTSTRAP_PHASE)
    verify_version_contains(destination, "--version", expected_fragment="git version")
    print(f"{GIT_BOOTSTRAP_PHASE}: ok -> {destination}", flush=True)


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
    result = run_installer_json(
        binary,
        [
            "--json",
            "--managed-dir",
            str(managed_dir),
            "--method",
            UV_TOOL_PHASE,
            "--id",
            UV_TOOL_ID,
            "--package",
            UV_TOOL_PACKAGE,
            "--python",
            UV_PYTHON_VERSION,
        ],
        attempts=DOWNLOAD_ATTEMPTS,
    )
    item = single_item(result)
    destination = require_installed(item, phase=UV_TOOL_PHASE)
    verify_version_contains(destination, "--version", expected_fragment="ruff ")
    print(f"{UV_TOOL_PHASE}: ok -> {destination}", flush=True)


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
            elif phase == GH_RELEASE_PHASE:
                phase_release_gh(binary, target_triple, workspace)
            elif phase == GIT_BOOTSTRAP_PHASE:
                phase_bootstrap_git(binary, workspace)
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
