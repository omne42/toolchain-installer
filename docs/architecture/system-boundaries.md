# 系统边界

## 目标

`toolchain-installer` 负责在调用方缺少开发工具链时，提供稳定、可验证、可集成的安装能力。它是独立安装基建，不承载调用方的业务协议、事件模型或目录约定。

## 顶层边界

- 二进制入口：`src/main.rs`
  - 负责启动 CLI，并把参数解析交给二进制专属模块。
- 库入口：`src/lib.rs`
  - 只做模块装配与公开导出，不承载流程细节。
- 安装域：`src/bootstrap/`、`src/plan/`、`src/installation/`、`src/managed_toolchain/`、`src/uv/`、`src/source_acquisition/`
  - 共同覆盖“确定装什么、从哪下载、如何安装、如何输出结果”。
- 平台域：`src/platform/`
  - 负责对 runtime 平台原语的安装域适配，包括进程执行适配和“宿主机探测 + OS 级系统包 recipe”组合。
- 契约域：`src/contracts/`、`src/error.rs`、`src/installer_runtime_config.rs`
  - 负责外部输入/输出、退出码、环境变量和运行期配置边界。
- Shared foundation 依赖：`../omne_foundation/crates/http-kit/`
  - 提供通用 HTTP client、bounded body read / preview、bounded response streaming、URL 校验 / 脱敏、untrusted outbound policy 与 endpoint 探测。
- Shared runtime 依赖：`../omne-runtime/crates/omne-integrity-primitives/`
  - 提供 `sha256` 解析、内容摘要计算与校验原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-host-info-primitives/`
  - 提供宿主 OS/arch 识别、canonical target triple 映射、target override 归一化、home 目录解析与目标可执行后缀原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-archive-primitives/`
  - 提供 archive/compression 格式识别、归档条目遍历和目标二进制提取原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-fs-primitives/`
  - 提供底层目录创建、暂存文件写入、权限设置、文件校验与原子替换原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-process-primitives/`
  - 提供宿主机命令探测、带输出捕获的命令执行、工作目录注入，以及命令路径解析 / 标准位置回退和 Unix 下对系统命令的 `sudo -n` 试探原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-system-package-primitives/`
  - 提供系统包管理器枚举、canonical 名称解析、安装 recipe 建模，以及按 OS 生成默认系统包安装配方的原语。
- 外部网关项目：`../toolchain-edge-gateway/`
  - 可选固定路由层，用于网络优化与反滥用；installer 只通过 `--gateway-base` 与其集成，不在本仓库内持有实现。

## 模块职责边界

- `bootstrap`
  - 面向“当前宿主机补齐基础工具链”的内置用例。
  - 只允许处理宿主机场景，不暴露跨目标平台语义。
- `plan`
  - 执行调用方给定的安装计划，并负责把原始 `method` 字符串归位成更明确的领域方法分类。
  - 只校验和编排，不拥有下载资产或解压细节。
- `installation`
  - 负责安装域编排：调用 archive runtime 提取目标二进制，再调用共享文件原语完成权限设置与目标路径落盘。
  - 不决定使用哪个来源或哪种安装方法。
- `source_acquisition`
  - 负责 installer 自己的下载候选建模、`gateway|canonical|mirror` 来源分类、GitHub Release 元数据抓取，以及外部网关资产路由拼装。
  - 不抽象成通用 HTTP foundation，也不承载 plan 编排、归档落盘或工具链布局策略。
- `managed_toolchain`
  - 负责围绕 `managed_dir` 的托管工具链环境编排：收敛 `uv` 工具目录、Python 目录和缓存目录，解析托管根目录策略，并执行 `uv`、`uv python install`、`uv tool install`。
  - 对上层接收的是显式托管工具链方法分发，而不是在领域内部继续解析原始方法字符串。
  - 不拥有 `uv` public release 元数据抓取和资产选择细节。
- `uv`
  - 负责 `uv` 自身的 public release 资产选择、摘要要求和 archive 安装适配。
  - 通过 installer 自己的 `source_acquisition` 模块消费 GitHub release 元数据和下载来源策略。
  - 不拥有 `managed_dir` 布局、Python mirror/index 策略或 plan 输出模型。
- `platform`
  - 负责对 runtime 平台原语做 installer 侧组合适配。
  - 例如把宿主机探测和 OS 级系统包 recipe 原语组合成“当前宿主机默认 recipe”。
  - 不关心上层 CLI 参数如何组织。
- `omne-host-info-primitives`
  - 负责宿主 OS/arch 识别、canonical target triple 映射、target override 归一化、home 目录解析与目标可执行后缀推断。
  - 不负责 `OMNE_DATA_DIR`、`TOOLCHAIN_INSTALLER_MANAGED_DIR`、`managed_dir` 布局或 installer plan 语义。
- `http-kit`
  - 负责通用 HTTP client、bounded body read / preview、bounded response streaming、URL 校验 / 脱敏、untrusted outbound policy 与 HTTP 可达性探测。
  - 不承载 GitHub release schema、下载来源分类、镜像 / 网关候选顺序或安装器资产命名。
- `omne-integrity-primitives`
  - 负责 `sha256:<hex>` 解析、原始 hex 输入解析、内容摘要计算与校验错误建模。
  - 不负责下载来源选择、release 元数据或安装落盘。
- `omne-archive-primitives`
  - 负责 `.tar.gz`、`.tar.xz`、`.zip` 的识别、归档条目遍历、条目路径归一化，以及按二进制名/工具名/hint 提取目标条目。
  - 不负责下载、权限设置、目标路径写入或安装计划。
- `omne-fs-primitives`
  - 负责通用目录创建、临时文件写入、flush/sync、Unix chmod、文件有效性校验和原子替换。
  - 不理解归档格式、二进制名称匹配或工具链安装计划。
- `omne-process-primitives`
  - 负责通用宿主机命令探测、带输出捕获的命令执行、工作目录注入、命令路径解析 / 标准位置回退，以及 Unix 下对 bare system command 的 `sudo -n` 试探。
  - 不负责包管理器配方、plan 语义、超时策略或安装领域错误码。
- `omne-system-package-primitives`
  - 负责系统包管理器枚举、canonical 名称解析、安装 recipe 建模，以及按 OS 生成默认系统包安装配方。
  - 不负责宿主机探测、plan method、tool/package 映射、结果 contract 或进程执行。
- `../toolchain-edge-gateway/`
  - 负责产品级边缘策略：固定路由重定向、国家限制和限流。
  - 不负责通用下载原语、归档/文件/进程原语，也不进入主 CLI 安装闭环。

## Worker 归位原则

- 当前 Worker 已拆分到 `../toolchain-edge-gateway/`
  - 它是 installer 的外部 edge adapter，不是 `omne_foundation` 或 `omne-runtime` 的共享能力。
- 不下沉到 shared crates
  - 因为它绑定 Cloudflare Worker 运行环境，并编码了 `git-for-windows`、`CN` 国家限制和限流策略。
- 当前已经拆仓
  - 现在由独立网关项目承载实现与测试；installer 仓库只保留集成边界与来源选择规则。

## 宿主与目标语义

- `host_triple`
  - 当前运行安装器的宿主平台，由 `omne-host-info-primitives` 自动探测。
- `target_triple`
  - 期望安装产物的目标平台；默认等于 `host_triple`。
- `bootstrap`
  - 只解决当前宿主机缺失工具链，因此要求 `target_triple == host_triple`。
- `plan.method=release`
  - 支持显式跨目标平台下载与落盘。
- `plan.method=system_package|apt|pip|uv|uv_python|uv_tool`
  - 只作用于当前宿主机；若 `target_triple != host_triple`，执行前直接报参数错误。

## 调用方边界

- 调用方只依赖 CLI 契约与 JSON 输出，不依赖日志文本。
- 调用方可以提供 plan，但不应把下载、校验、安装细节复制到自身仓库。
- `omne-agent` 是一个调用方，但不是安装器架构的一部分。

## 非目标

- 不提供任意 URL 下载代理。
- 不托管私有二进制分发服务。
- 不耦合单一业务仓库的线程模型、目录结构或事件系统。

## 继续阅读

- CLI 表面：`../contracts/cli-surface.md`
- plan 约束：`../contracts/install-plan-contract.md`
- 源码树职责：`source-layout.md`
- Worker 归位：`worker-gateway-placement.md`
- 外部网关集成：`../operations/external-gateway-integration.md`
