# 源码布局

## Rust 入口

- `src/main.rs`
  - 二进制入口。只负责调用 CLI 解析与执行，不承载业务规则。
- `src/cli.rs`
  - 二进制专属 CLI 参数模型、plan 文件读取和命令分发。
- `src/lib.rs`
  - 库入口。只做模块装配与公开导出。

## 顶层目录与文件

- `src/application/`
  - `bootstrap_use_case.rs`：`bootstrap` 用例编排，负责调用内置工具领域策略并汇总输出。
  - `execution_context.rs`：共享执行上下文 builder，集中初始化 host/target、managed_dir、runtime config 和 HTTP client。
  - `install_plan_use_case.rs`：plan 用例编排，负责驱动 plan 校验、执行和结果归并。
  - `mod.rs`：应用编排层汇总。
- `src/artifact/`
  - `install_source.rs`：内部安装来源结果模型，供 bootstrap / managed_toolchain 这些内部安装流程归并结果使用。
  - `mod.rs`：artifact 内部域汇总。
- `src/builtin_tools/`
  - `builtin_tool_selection.rs`：内置工具默认选择、输入归一化与支持集判定。
  - `public_release_asset_installation.rs`：`gh` / `git` public release 资产选择与安装适配。
  - `mod.rs`：内置工具领域汇总。
- `src/contracts/`
  - `execution_request.rs`：`ExecutionRequest` 与 `BootstrapCommand` 输入契约。
  - `bootstrap_result.rs`：稳定 JSON 输出、状态与 schema 版本。
  - `install_plan_contract.rs`：安装 plan schema 与条目结构。
  - `mod.rs`：契约域汇总导出，并向 crate 内部暴露失败 `BootstrapItem` 构造辅助。
- `src/external_gateway/`
  - `asset_routing.rs`：installer 对外部 edge gateway 的资产路由拼装，以及 `git-for-windows` release URL/asset 到 gateway 候选的推断。
  - `mod.rs`：外部网关集成域汇总。
- `src/download_sources.rs`
  - installer 自有的下载来源选择辅助，负责 `gateway|canonical|mirror` 候选展开与结果来源种类映射。
  - 不承载 GitHub API client、下载执行、摘要校验或安装编排。
- `src/install_plan/`
  - `resolved_plan_item.rs`：把外部 `InstallPlanItem` DTO 收敛成内部强类型 `ResolvedPlanItem`，归一化方法级字段组合。
  - `item_destination_resolution.rs`：plan item 的有效目标路径解析，以及 release 默认落点计算。
  - `item_method_dispatch.rs`：消费 `ResolvedPlanItem` 做领域方法分发，不再直接读取外部弱类型 DTO。
  - `release_item_execution.rs`：`plan.method=release` 的下载、摘要校验与安装执行。
  - `archive_tree_release_item_execution.rs`：`plan.method=archive_tree_release` 的整目录归档下载、摘要校验与 staging+replace 安装执行。
  - `system_package_item_execution.rs`：`plan.method=system_package|apt` 的系统包执行，消费 runtime system-package primitives 生成 recipe。
  - `pip_item_execution.rs`：`plan.method=pip` 的 Python/pip 执行。
  - `npm_global_item_execution.rs`：`plan.method=npm_global` 的 npm/pnpm/bun 全局 CLI 安装执行，以及 Windows bun launcher 补齐。
  - `workspace_package_item_execution.rs`：`plan.method=workspace_package` 的现有 JS workspace 目录依赖安装执行。
  - `cargo_install_item_execution.rs`：`plan.method=cargo_install` 的 Rust CLI 安装执行，覆盖 crate/package 与本地路径两类来源。
  - `rustup_component_item_execution.rs`：`plan.method=rustup_component` 的 Rust toolchain component 安装执行，并尽量解析组件对应二进制落点。
  - `go_install_item_execution.rs`：`plan.method=go_install` 的 Go CLI 安装执行，覆盖 package spec 与本地路径两类来源。
  - `install_plan_validation.rs`：plan schema、宿主/目标约束校验，并驱动 DTO -> `ResolvedPlanItem` 的收敛。
  - `plan_method.rs`：plan 方法分类、宿主绑定判定与领域方法映射。
  - `mod.rs`：install plan 领域汇总。
- `src/managed_toolchain/`
  - `managed_root_dir.rs`：托管工具链根目录解析与默认 `~/.omne_data/toolchain/<target>/bin` 布局。
  - `managed_environment_layout.rs`：`managed_dir` 周边的 `uv` 工具目录、Python 目录、缓存目录、环境变量和可执行目标路径布局。
  - `bootstrap_item_construction.rs`：托管工具链安装成功后的 `BootstrapItem` 组装与 detail 归并。
  - `source_candidate_attempts.rs`：按候选安装来源顺序尝试执行，并聚合失败信息。
  - `managed_uv_host_execution.rs`：托管 `uv` 子流程执行辅助，负责在调用前移除继承的宿主 `UV_*` 环境变量，再注入 installer 自己的托管 `uv` 环境。
  - `managed_uv_installation.rs`：确保托管 `uv` 已安装，并处理 `plan.method=uv` 的结果归并。
  - `version_probe.rs`：托管 `uv`、托管 Python 与 bootstrap 健康探针共享的 `--version` 超时探测与 Python 版本匹配。
  - `uv_public_release_installation.rs`：托管 `uv` 的 public release 资产选择、摘要要求与 archive 安装适配；`bootstrap` 也复用它补齐缺失的 `uv`。
  - `managed_python_executable_discovery.rs`：在 `managed_dir` 中定位满足版本要求的托管 Python 可执行文件。
  - `uv_python_installation.rs`：执行 `uv python install`，并委托 Python 发现逻辑解析托管可执行落点。
  - `uv_tool_installation.rs`：执行 `uv tool install`，并把工具二进制收敛到 `managed_dir`。
  - `uv_installation_source_candidates.rs`：生成 `uv_python`/`uv_tool` 的安装来源候选，并按可达性调整尝试顺序。
  - `mod.rs`：托管工具链领域汇总。

## Shared Workspace Dependencies

- `../omne_foundation/crates/http-kit/`
  - shared foundation 的 HTTP client、bounded body read / preview、bounded response streaming、URL 校验 / 脱敏、untrusted outbound policy 与 endpoint 探测。
  - `toolchain-installer` 不在 `http-kit` 内部放 GitHub API schema 或下载来源策略；这些逻辑分别归位到 `github-kit` 与 installer 自身。
- `../omne_foundation/crates/github-kit/`
  - shared foundation 的纯 GitHub API client 能力：repository 标识校验、latest release URL 构造、多 API base 回退与 release metadata DTO。
  - `toolchain-installer` 复用它请求 release metadata，但继续在本仓保留来源候选顺序和资产选择策略。
- `../omne-runtime/crates/omne-artifact-install-primitives/`
  - shared runtime 的 artifact 下载候选执行、SHA-256 校验、direct binary 安装、archive binary 安装与 archive tree staging+replace 原语。
  - `toolchain-installer` 不再自己维护重复的 release/archive 下载、校验和落盘/解压安装链路。
- `../omne-runtime/crates/omne-integrity-primitives/`
  - shared runtime 的通用 `sha256` 解析、内容摘要与校验原语。
  - `toolchain-installer` 不再自己维护摘要解析和校验逻辑。
- `../omne-runtime/crates/omne-host-info-primitives/`
  - shared runtime 的宿主平台识别、目标三元组映射、target override 归一化、home 目录解析与目标可执行后缀原语。
  - `toolchain-installer` 不再自己维护宿主 OS/arch 到 target triple 的映射，也不再维护独立的 target override 解析壳。
- `../omne-runtime/crates/omne-archive-primitives/`
  - shared runtime 的 archive/compression 能力：`.tar.gz`、`.tar.xz`、`.zip` 识别，归档条目遍历，按二进制名/工具名/hint 匹配并提取目标条目。
  - `toolchain-installer` 不再维护自己的归档格式识别和条目匹配逻辑。
- `../omne-runtime/crates/omne-fs-primitives/`
  - shared runtime 的通用目录创建、临时文件写入、flush/sync、Unix chmod、非空/可执行校验与原子替换原语。
  - `toolchain-installer` 不再维护自己的临时文件命名与目标替换细节。
- `../omne-runtime/crates/omne-process-primitives/`
  - shared runtime 的通用命令探测、带输出捕获的宿主机命令执行、host recipe 执行、工作目录注入、默认 `sudo` 模式推断、命令路径解析 / 标准位置回退，以及 Unix 下针对系统命令的 `sudo -n` 试探原语。
  - `toolchain-installer` 直接消费这些原语，不再维护本地 host recipe 执行适配层。
- `../omne-runtime/crates/omne-system-package-primitives/`
  - shared runtime 的通用系统包管理器枚举、canonical 名称解析、安装 recipe 建模，以及按 OS 生成默认系统包安装配方的原语。
  - `toolchain-installer` 不再维护自己的 system package manager 类型和默认包管理器顺序。
- `../toolchain-edge-gateway/`
  - 外部可选边缘网关项目，负责 Cloudflare Worker 固定路由、国家限制与限流。
  - `toolchain-installer` 只保留 `--gateway-base` 集成与来源候选选择。

## 顶层文件

- `src/error.rs`
  - 错误类型、退出码与统一结果类型。
- `src/installer_runtime_config.rs`
  - installer 运行期配置与环境变量归一化，消费 `ExecutionRequest` 组装运行时策略。
  - 内部进一步拆成 `GitHubReleasePolicy`、`DownloadSourcePolicy`、`DownloadPolicy`、`PackageIndexPolicy`、`PythonMirrorPolicy`、`GatewayRoutingPolicy`，避免 GitHub API、镜像候选、索引、gateway、下载限制继续揉在一个平铺 struct 里。
- `src/plan_items.rs`
  - `install_plan` 与 `managed_toolchain` 共享的强类型安装项模型，例如 `ResolvedPlanItem`、`UvPythonPlanItem`、`UvToolPlanItem`。
  - 只承载共享领域数据，不承载 plan 校验、方法归一化或执行编排逻辑。

## 测试与辅助目录

- `src/lib_tests.rs`
  - 库内单元测试。
- `tests/e2e_cli.rs`
  - CLI 端到端行为测试。
- `tests/docs_system.rs`
  - 文档系统入口与关键路径存在性检查。
- `scripts/install_smoke.py`
  - GitHub-hosted Linux、macOS、Windows 上的真实安装 smoke 脚本；由 workflow 调用，也可本地按 phase 复用。
- `tools/source-layout-check/`
  - 独立 Rust 检查器 crate，供 `.githooks/pre-commit` 与 CI 校验 `docs/architecture/source-layout.md` 是否与 `src/` 文件树双向一致，且顶层 `src/*/` 目录条目按字母序排列。
  - 同时校验顶层 crate 模块依赖方向，阻止 `managed_toolchain` 之类的领域模块反向依赖 `application` 或 `install_plan` 之类的上层模块。
- `.github/actions/checkout-shared-deps/`
  - CI 复用的 shared repo checkout 入口；把 `omne_foundation` 和 `omne-runtime` 拉到 sibling 目录，满足本仓库的 path dependency 布局。
- `examples/`
  - 可执行的 plan 示例。

## 布局约束

- 本文件中的所有顶层 `src/*/` 目录条目必须按字母序排列。
- 按领域拆目录，不按技术动作堆砌杂项文件。
- 文件名必须能直接透露职责，例如 `system_package_item_execution.rs`、`install_plan_use_case.rs`、`execution_context.rs`、`plan_method.rs`。
- 二进制专属代码放在二进制入口侧，不把 CLI 噪声带入库 API。
- 当一个文件同时承担多个领域职责时再拆分；不要为了“层数好看”制造空模块。

## 变更触发器

- 新增顶层领域目录时，必须同步更新本文件。
- 文件职责变化但文件名不再准确时，优先重命名，再考虑补注释。
- 若新增测试类型，应把入口位置补充到本文件，避免测试资产藏在隐蔽路径。
