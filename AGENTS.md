# toolchain-installer AGENTS Map

这个文件只做导航，不承载完整事实。仓库内受版本控制的 Markdown 与示例 plan 文件才是记录系统。

## 先看哪里

- 外部概览：`README.md`
- 文档系统入口：`docs/docs-system-map.md`
- 系统边界：`docs/architecture/system-boundaries.md`
- Worker 归位：`docs/architecture/worker-gateway-placement.md`
- 源码布局：`docs/architecture/source-layout.md`
- CLI 参数与输出：`docs/contracts/cli-surface.md`
- plan schema 与方法约束：`docs/contracts/install-plan-contract.md`
- Python 3.13.12 + uv + ruff + mypy 引导：`docs/guides/python-toolchain-bootstrap.md`
- 安全边界：`docs/operations/security-boundaries.md`
- 外部网关集成：`docs/operations/external-gateway-integration.md`
- 质量门槛与文档维护：`docs/operations/quality-and-doc-maintenance.md`
- 当前路线图与文档债务：`docs/plans/delivery-roadmap.md`、`docs/plans/documentation-tech-debt.md`
- 外部网关项目：`../toolchain-edge-gateway/`

## 代码地图

- `src/main.rs`：二进制入口，只负责 CLI 启动。
- `src/bootstrap/`：内置 `bootstrap` 用例与二进制专属 CLI 参数解析。
  这里的 `builtin_tools/` 已按流程、工具目录、public release 安装与网关候选拆分。
- `src/plan/`：安装 plan 校验与执行。
  这里通过 `plan_method.rs` 把原始 `method` 字符串归位到更明确的领域方法分类，并把 `install_plan_validation.rs`、`install_plan_execution.rs`、`item_destination_resolution.rs`、`item_method_dispatch.rs`、`release_item_execution.rs`、`system_package_item_execution.rs`、`pip_item_execution.rs` 拆开承载。
- `src/platform/`：进程执行与系统包适配。
  这里的进程执行是适配层；低层命令探测与执行原语在 runtime。
  系统包管理器枚举、别名与默认安装 recipe 已下沉到 vendored runtime snapshot：`vendor/omne-runtime/crates/omne-system-package-primitives/`。
- `src/installation/`：安装域适配层，调用共享 archive/runtime 能力提取目标二进制，并调用共享文件原语完成原子落盘。
- `src/managed_toolchain/`：围绕 `managed_dir` 的托管工具链环境领域，负责 `uv`、`uv python install`、`uv tool install` 的环境布局与编排。
  这里进一步拆成 `managed_root_dir.rs`、`managed_environment_layout.rs`、`managed_uv_installation.rs`、`managed_python_executable_discovery.rs`、`uv_python_installation.rs`、`uv_tool_installation.rs`、`uv_installation_source_candidates.rs`、`source_candidate_attempts.rs`、`bootstrap_item_construction.rs`。
- `src/uv/`：`uv` public release 资产选择与安装细节。
- `src/source_acquisition/`：installer 自有的来源获取领域，承载下载候选、GitHub release 元数据、外部网关路由与来源种类映射。
- `src/contracts/`：稳定输入/输出契约，进一步拆成 bootstrap request/result、install plan contract 与内部 source model。
- `src/error.rs`：错误类型与退出码。
- `src/installer_runtime_config.rs`：installer 运行期配置与环境变量收敛。
- `vendor/http-kit/`：从 shared foundation vendored 的 HTTP 通用能力快照；不承载 installer 自己的 GitHub release schema 与来源候选策略。
- `vendor/omne-runtime/crates/omne-host-info-primitives/`：从 shared runtime vendored 的宿主平台识别、target triple 映射、target override 归一化、home 目录解析与目标可执行后缀原语。
- `vendor/omne-runtime/crates/omne-integrity-primitives/`：从 shared runtime vendored 的通用 `sha256` 解析、内容摘要与校验原语。
- `vendor/omne-runtime/crates/omne-archive-primitives/`：从 shared runtime vendored 的 archive/compression 格式识别、归档条目遍历与目标二进制提取原语。
- `vendor/omne-runtime/crates/omne-fs-primitives/`：从 shared runtime vendored 的低层目录创建、临时文件写入、权限校验与原子替换原语。
- `vendor/omne-runtime/crates/omne-process-primitives/`：从 shared runtime vendored 的宿主机命令探测、带输出捕获的命令执行与 Unix `sudo -n` 试探原语。
- `../toolchain-edge-gateway/`：外部边缘网关项目；installer 只通过 `--gateway-base` 与其集成。

## 修改规则

- 行为变化必须同 PR 更新对应文档，不把事实留在聊天记录里。
- 文件名必须能从路径判断职责；不要新增语义模糊的 `misc`、`utils`、`index` 式文档。
- CLI 表面变化更新 `docs/contracts/cli-surface.md`。
- plan 方法、字段或来源选择变化更新 `docs/contracts/install-plan-contract.md` 与 `docs/references/source-selection-rules.md`。
- installer 对外部网关的集成边界变化更新 `docs/operations/external-gateway-integration.md`。
- Worker 归位、拆分策略或边界变化更新 `docs/architecture/worker-gateway-placement.md`。
- 新增源码目录或职责迁移时更新 `docs/architecture/source-layout.md`。
- 新增示例 plan 时更新 `docs/references/example-plan-files.md`。

## 验证

- `cargo fmt --all`
- `cargo check --all-targets`
- `cargo test --all-targets`
- 如修改安装链路、平台分支或 workflow 验证，再运行 `cargo build --release` 与 `python3 scripts/install_smoke.py --binary ./target/release/toolchain-installer --phase ...`
- 如修改外部网关项目，再运行 `cd ../toolchain-edge-gateway && npm test`

`tests/docs_system.rs` 会检查 installer 仓库内的文档入口和关键文件是否仍然存在。
