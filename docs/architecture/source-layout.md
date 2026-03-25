# 源码布局

## Rust 入口

- `src/main.rs`
  - 二进制入口。只负责调用 CLI 解析与执行，不承载业务规则。
- `src/lib.rs`
  - 库入口。只做模块装配与公开导出。

## 领域目录

- `src/bootstrap/`
  - `builtin_tools/`
    - `bootstrap_execution.rs`：默认 `bootstrap` 执行流程与内置工具安装编排。
    - `builtin_tool_selection.rs`：内置工具默认选择、输入归一化与支持集判定。
    - `public_release_asset_installation.rs`：`gh` / `git` public release 资产选择与安装适配。
    - `mod.rs`：内置 bootstrap 领域汇总。
  - `cli.rs`：二进制专属 CLI 参数模型与命令分发。
  - `mod.rs`：`bootstrap` 领域汇总。
- `src/plan/`
  - `item_destination_resolution.rs`：plan item 的有效目标路径解析，以及 release 默认落点计算。
  - `item_method_dispatch.rs`：plan item 领域方法分发与未知方法兜底结果组装。
  - `release_item_execution.rs`：`plan.method=release` 的下载、摘要校验与安装执行。
  - `system_package_item_execution.rs`：`plan.method=system_package|apt` 的系统包执行，消费 runtime system-package primitives 生成 recipe。
  - `pip_item_execution.rs`：`plan.method=pip` 的 Python/pip 执行。
  - `install_plan_validation.rs`：plan schema、字段组合与宿主/目标约束校验。
  - `install_plan_execution.rs`：plan 总执行编排、宿主/目标初始化和结果汇总。
  - `plan_method.rs`：plan 方法分类、宿主绑定判定与领域方法映射。
  - `mod.rs`：`plan` 领域汇总。
- `src/platform/`
  - `process_runner.rs`：平台域进程执行适配层，委托 runtime process primitives 做命令探测与执行。
  - `mod.rs`：平台领域汇总。
- `src/installation/`
  - `archive_binary.rs`：安装域适配层，委托 archive runtime 提取目标二进制，并委托共享文件原语完成目标文件原子落盘。
  - `mod.rs`：安装领域汇总。
- `src/managed_toolchain/`
  - `managed_root_dir.rs`：托管工具链根目录解析与默认 `~/.omne_data/toolchain/<target>/bin` 布局。
  - `managed_environment_layout.rs`：`managed_dir` 周边的 `uv` 工具目录、Python 目录、缓存目录、环境变量和可执行目标路径布局。
  - `bootstrap_item_construction.rs`：托管工具链安装成功后的 `BootstrapItem` 组装与 detail 归并。
  - `source_candidate_attempts.rs`：按候选安装来源顺序尝试执行，并聚合失败信息。
  - `managed_uv_installation.rs`：确保托管 `uv` 已安装，并处理 `plan.method=uv` 的结果归并。
  - `managed_python_executable_discovery.rs`：在 `managed_dir` 中定位满足版本要求的托管 Python 可执行文件。
  - `uv_python_installation.rs`：执行 `uv python install`，并委托 Python 发现逻辑解析托管可执行落点。
  - `uv_tool_installation.rs`：执行 `uv tool install`，并把工具二进制收敛到 `managed_dir`。
  - `uv_installation_source_candidates.rs`：生成 `uv_python`/`uv_tool` 的安装来源候选，并按可达性调整尝试顺序。
  - `mod.rs`：托管工具链领域汇总。
- `src/uv/`
  - `release_installation.rs`：`uv` public release 资产选择、摘要要求与 archive 安装适配。
  - `mod.rs`：`uv` 领域汇总。
- `src/source_acquisition/`
  - `download_candidates.rs`：installer 自身拥有的下载候选建模与 `gateway|canonical|mirror` 顺序生成。
  - `download_transfer.rs`：受限响应体流式写入适配。
  - `github_release_metadata.rs`：GitHub Release API DTO 与 latest release 元数据抓取适配。
  - `gateway_asset_routing.rs`：外部网关资产路由拼装与 git release URL 推断。
  - `download_source_kind_mapping.rs`：下载候选来源种类到 installer 输出种类的映射。
  - `mod.rs`：来源获取领域汇总。

## 外部基础依赖

- `../omne_foundation/crates/http-kit/`
  - 通用 HTTP client、bounded body read / preview、URL 校验 / 脱敏、untrusted outbound policy 与 endpoint 探测。
  - `toolchain-installer` 不再在 `http-kit` 内部放 GitHub release DTO 或下载来源策略；这些逻辑留在 installer 自身。
- `../omne-runtime/crates/omne-integrity-primitives/`
  - 通用 `sha256` 解析、内容摘要与校验原语。
  - `toolchain-installer` 不再自己维护摘要解析和校验逻辑。
- `../omne-runtime/crates/omne-host-info-primitives/`
  - 通用宿主平台识别、目标三元组映射、target override 归一化、home 目录解析与目标可执行后缀原语。
  - `toolchain-installer` 不再自己维护宿主 OS/arch 到 target triple 的映射，也不再维护独立的 target override 解析壳。
- `../omne-runtime/crates/omne-archive-primitives/`
  - 通用 archive/compression 能力：`.tar.gz`、`.tar.xz`、`.zip` 识别，归档条目遍历，按二进制名/工具名/hint 匹配并提取目标条目。
  - `toolchain-installer` 不再维护自己的归档格式识别和条目匹配逻辑。
- `../omne-runtime/crates/omne-fs-primitives/`
  - 通用目录创建、临时文件写入、flush/sync、Unix chmod、非空/可执行校验与原子替换原语。
  - `toolchain-installer` 不再维护自己的临时文件命名与目标替换细节。
- `../omne-runtime/crates/omne-process-primitives/`
  - 通用命令探测、带输出捕获的宿主机命令执行，以及 Unix 下针对系统命令的 `sudo -n` 试探原语。
  - `toolchain-installer` 不再维护自己的命令探测与提权执行细节。
- `../omne-runtime/crates/omne-system-package-primitives/`
  - 通用系统包管理器枚举、别名解析、安装 recipe 建模，以及按 OS / 当前宿主机生成默认系统包安装配方的原语。
  - `toolchain-installer` 不再维护自己的 system package manager 类型和默认包管理器顺序。
- `../toolchain-edge-gateway/`
  - 外部可选边缘网关项目，负责 Cloudflare Worker 固定路由、国家限制与限流。
  - `toolchain-installer` 只保留 `--gateway-base` 集成与来源候选选择。

## 契约与基础类型

- `src/contracts/`
  - `bootstrap_request.rs`：bootstrap/plan 共享输入契约。
  - `bootstrap_result.rs`：稳定 JSON 输出、状态与 schema 版本。
  - `install_source.rs`：安装结果内部来源模型。
  - `install_plan_contract.rs`：安装 plan schema 与条目结构。
  - `mod.rs`：契约域汇总导出。
- `src/error.rs`
  - 错误类型、退出码与统一结果类型。
- `src/installer_runtime_config.rs`
  - installer 运行期配置与环境变量归一化。

## 测试与辅助目录

- `src/lib_tests.rs`
  - 库内单元测试。
- `tests/e2e_cli.rs`
  - CLI 端到端行为测试。
- `tests/docs_system.rs`
  - 文档系统入口与关键路径存在性检查。
- `scripts/install_smoke.py`
  - GitHub-hosted Linux、macOS、Windows 上的真实安装 smoke 脚本；由 workflow 调用，也可本地按 phase 复用。
- `examples/`
  - 可执行的 plan 示例。

## 布局约束

- 按领域拆目录，不按技术动作堆砌杂项文件。
- 文件名必须能直接透露职责，例如 `system_package_item_execution.rs`、`install_plan_execution.rs`、`plan_method.rs`。
- 二进制专属代码放在二进制入口侧，不把 CLI 噪声带入库 API。
- 当一个文件同时承担多个领域职责时再拆分；不要为了“层数好看”制造空模块。

## 变更触发器

- 新增顶层领域目录时，必须同步更新本文件。
- 文件职责变化但文件名不再准确时，优先重命名，再考虑补注释。
- 若新增测试类型，应把入口位置补充到本文件，避免测试资产藏在隐蔽路径。
