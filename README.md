# toolchain-installer

一个通用、可复用的工具链安装器，用于在宿主机缺少开发工具链时提供稳定、可验证、可集成的安装能力。`omne-agent` 是调用方之一，但不是唯一调用方。

## 仓库提供什么

- 稳定 CLI：`toolchain-installer bootstrap [options]`
- 通用安装 plan 执行能力：`release`、`archive_tree_release`、`system_package`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install`、`uv`、`uv_python`、`uv_tool`
- 调用方无关的 JSON 输出契约
- 官方来源优先、镜像回退和可达性探测
- 可选外部固定路由网关集成接口：`--gateway-base`

## 关键约束

- `bootstrap` 只解决当前宿主机的工具链补齐，不支持跨目标平台安装。
- 只有 `release` 与 `archive_tree_release` 支持显式跨目标平台下载。
- `system_package`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install`、`uv`、`uv_python`、`uv_tool` 都是宿主机方法。
- `pip` 只有在默认首选解释器命令不存在时才会回退到后续候选；若首选解释器已经执行 `-m pip install` 并失败，installer 会直接报错，不会静默装到另一个 Python 环境。
- 未显式传 `--managed-dir` 时，默认托管目录是 `~/.omne_data/toolchain/<target>/bin`。
- `release` 的相对 `destination` 解析到 `managed_dir` 下，并拒绝 `..` 路径逃逸。
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
