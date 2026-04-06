# toolchain-installer

一个通用、可复用的工具链安装器，用于在宿主机缺少开发工具链时提供稳定、可验证、可集成的安装能力。`omne-agent` 是调用方之一，但不是唯一调用方。

## 仓库提供什么

- 稳定 CLI：`toolchain-installer bootstrap [options]`
- 通用安装 plan 执行能力：`release`、`archive_tree_release`、`system_package`、`apt`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install`、`uv`、`uv_python`、`uv_tool`
- 调用方无关的 JSON 输出契约
- 官方来源优先、镜像回退和可达性探测
- 可选外部固定路由网关集成接口：`--gateway-base`

## 关键约束

- `bootstrap` 只解决当前宿主机的工具链补齐，不支持跨目标平台安装。
- 如果 `managed_dir` 里已有同名内置工具但健康检查失败，`bootstrap` 会优先修复托管副本，而不会因为宿主 PATH 上碰巧存在健康同名命令就把结果降级成 `present`。
- `bootstrap` 只有在本次安装后的目标工具再次通过同等级健康检查后，才会返回 `status=installed`；下载、解压或系统包命令本身成功但产物仍不可用时，会返回失败而不是误报成功。
- 只有 `release` 与 `archive_tree_release` 支持显式跨目标平台下载。
- `system_package`、`apt`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install`、`uv`、`uv_python`、`uv_tool` 都是宿主机方法。
- `apt` 是 `system_package` 域下的显式 alias：它固定执行 canonical `apt-get`，并且只接受可选 `manager=apt-get`。
- `pip` 表达的是“把包安装进选定 Python 环境”这类宿主环境变更，不承诺 installer 自己拥有的托管 `destination` 或可重放 artifact 坐标。
- `pip` 只有在默认首选解释器命令不存在时才会回退到后续候选；若首选解释器已经执行 `-m pip install` 并失败，installer 会直接报错，不会静默装到另一个 Python 环境。
- `system_package`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install` 以及 bootstrap 的系统包 / `pip install uv` fallback 都会带 hard timeout；默认是 `900` 秒，可通过 `--host-recipe-timeout-seconds` 或 `TOOLCHAIN_INSTALLER_HOST_RECIPE_TIMEOUT_SECONDS` 覆盖。
- `uv_python` 与 `uv_tool` 的托管 `uv` 安装子进程会带有界 stdout/stderr 捕获和 hard timeout；默认是 `900` 秒，可通过 `TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS` 覆盖。
- 未显式传 `--managed-dir` 时，默认托管目录是 `~/.omne_data/toolchain/<target>/bin`。
- CLI 会先把 `TOOLCHAIN_INSTALLER_MANAGED_DIR`、`OMNE_DATA_DIR` 等 env fallback 收口进 `ExecutionRequest`；纯库调用若需要相同行为，必须显式调用 `ExecutionRequest::with_process_environment_fallbacks()`，否则不会再被这两个环境变量偷偷改写 `managed_dir`。
- `release` 的相对 `destination` 解析到 `managed_dir` 下，并拒绝 `..` 路径逃逸。
- 非 Windows 宿主即使显式把 `target_triple` 设为 Windows，也不会接受 `C:\tools\demo.exe` 这类 Windows 绝对 `destination`；installer 只接受当前宿主机真实可落盘的绝对路径语义。
- Windows 托管 `git` 更新会把 `git-portable/` payload 切换和 `git.cmd` launcher 重写放进同一事务；launcher 写失败时会回滚到旧 payload，不留下半更新状态。
- 失败项除了 `detail` 外，还会返回机器可读的 `error_code`。

## 文档入口

这个仓库采用“短入口 + 分层事实文档”的文档系统。先看这些文件：

- `AGENTS.md`：执行者地图
- `docs/README.md`：文档入口
- `docs/docs-system-map.md`：文档系统入口与维护规则
- `docs/architecture/system-boundaries.md`：系统边界
- `docs/architecture/source-layout.md`：源码布局
- `docs/contracts/cli-surface.md`：CLI 契约
- `docs/contracts/install-plan-contract.md`：plan 契约
- `docs/guides/python-toolchain-bootstrap.md`：Python 3.13.12 + uv + ruff + mypy 引导
- `docs/operations/security-boundaries.md`：安全边界
- `docs/operations/external-gateway-integration.md`：外部网关集成边界
- `docs/plans/delivery-roadmap.md`：当前路线图

## 最小验证

```bash
cargo fmt --all
cargo check --all-targets
cargo test --all-targets
python3 scripts/install_smoke.py
```

若同时修改外部网关项目，再额外执行：

```bash
cd ../toolchain-edge-gateway && npm test
```
