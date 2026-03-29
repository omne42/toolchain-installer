# toolchain-installer AGENTS Map

这个文件只做导航，不承载完整事实。仓库内受版本控制的 Markdown 与示例 plan 文件才是记录系统。

## 先看哪里

- 外部概览：`README.md`
- 文档入口：`docs/README.md`
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
- `src/cli.rs`：二进制专属 CLI 参数模型、plan 文件读取与命令分发。
- `src/application/`：顶层用例编排。
  这里收口 `bootstrap_use_case.rs`、`install_plan_use_case.rs` 与 `execution_context.rs`，负责装配执行上下文、调用领域模块并汇总稳定输出。
- `src/builtin_tools/`：内置 `bootstrap` 工具域。
  这里承载默认工具选择，以及 `git` / `gh` 的 public release 资产选择与安装适配。
- `src/install_plan/`：安装 plan 领域。
  这里负责 `method` 归位、DTO -> `ResolvedPlanItem` 收敛、目标路径解析、冲突校验、方法分发，以及各 plan method 的执行实现。
- `src/artifact/`：内部 artifact 安装结果域。
  这里的 `install_source.rs` 只承载内部安装来源结果，不是外部 contract。
- `src/managed_toolchain/`：围绕 `managed_dir` 的托管工具链环境领域，负责 `uv`、`uv python install`、`uv tool install` 的环境布局与编排。
  这里进一步拆成 `managed_root_dir.rs`、`managed_environment_layout.rs`、`managed_uv_installation.rs`、`managed_uv_host_execution.rs`、`uv_public_release_installation.rs`、`managed_python_executable_discovery.rs`、`uv_python_installation.rs`、`uv_tool_installation.rs`、`uv_installation_source_candidates.rs`、`source_candidate_attempts.rs`、`bootstrap_item_construction.rs`、`version_probe.rs`。
  `bootstrap` 补齐 `uv` 时也直接复用这里的 public release 安装能力，不再额外维护一个顶层 `uv` 域。
- `src/github_release_metadata.rs`：installer 对 shared `github-kit` 的窄适配层。
  这里只负责把运行期 GitHub API 配置收敛成统一的 latest release metadata 调用，不承载资产选择或来源候选顺序。
- `src/download_sources.rs`：installer 自有的下载来源选择域，只承载下载候选构造与来源种类映射；GitHub release metadata client 已下沉到 foundation 的 `github-kit`。
- `src/external_gateway/`：installer 对外部 edge gateway 的内部集成域。
  这里承载 gateway 资产路由拼装与 `git-for-windows` release URL 推断，不和通用来源获取逻辑混层。
- `src/contracts/`：稳定输入/输出契约，只承载 bootstrap request/result 与 install plan contract。
- `src/plan_items.rs`：`install_plan` 与 `managed_toolchain` 共享的强类型安装项模型。
- `src/error.rs`：错误类型与退出码。
- `src/installer_runtime_config.rs`：installer 运行期配置与环境变量收敛。
  这里已经拆成 `github_releases`、`download_sources`、`download`、`package_indexes`、`python_mirrors`、`gateway` 这些内部策略子结构，不再让所有产品配置揉成一个平铺大对象。
- `../omne_foundation/crates/http-kit/`：shared foundation 的 HTTP 通用能力；不承载 installer 自己的 GitHub release schema 与来源候选策略。
- `../omne_foundation/crates/github-kit/`：shared foundation 的纯 GitHub API client 能力；负责 latest release metadata 获取，不承载 installer 的来源候选顺序或资产选择策略。
- `../omne-runtime/crates/omne-artifact-install-primitives/`：shared runtime 的 artifact 下载候选执行、SHA 校验、binary/tree 安装管道原语。
- `../omne-runtime/crates/omne-host-info-primitives/`：shared runtime 的宿主平台识别、target triple 映射、target override 归一化、home 目录解析与目标可执行后缀原语。
- `../omne-runtime/crates/omne-integrity-primitives/`：shared runtime 的通用 `sha256` 解析、内容摘要与校验原语。
- `../omne-runtime/crates/omne-archive-primitives/`：shared runtime 的 archive/compression 格式识别、归档条目遍历与目标二进制提取原语。
- `../omne-runtime/crates/omne-fs-primitives/`：shared runtime 的低层目录创建、临时文件写入、权限校验与原子替换原语。
- `../omne-runtime/crates/omne-process-primitives/`：shared runtime 的宿主机命令探测、host recipe 执行、默认 `sudo` 模式推断、带输出捕获的命令执行与 Unix `sudo -n` 试探原语。
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
