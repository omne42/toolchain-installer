# 系统边界

## 目标

`toolchain-installer` 负责在调用方缺少开发工具链时，提供稳定、可验证、可集成的安装能力。它是独立安装基建，不承载调用方的业务协议、事件模型或目录约定。

## 顶层边界

- 二进制入口：`src/main.rs`
  - 负责启动 CLI，并把参数解析交给二进制专属模块。
- 二进制专属 CLI：`src/cli.rs`
  - 负责 CLI 参数模型、文件输入读取和对库用例的命令分发，不承载安装策略。
- 库入口：`src/lib.rs`
  - 只做模块装配与公开导出，不承载流程细节。
- 应用编排域：`src/application/`
  - 负责 bootstrap / install plan 用例编排、执行上下文初始化，以及把领域执行结果汇总成稳定输出。
- 领域策略域：`src/builtin_tools/`、`src/install_plan/`、`src/managed_toolchain/`、`src/download_sources.rs`、`src/plan_items.rs`
  - 共同覆盖“确定装什么、从哪下载、如何安装”的领域策略与共享模型；不负责 CLI 解析或运行上下文装配。
- artifact 内部域：`src/artifact/`
  - 承载内部 artifact 安装结果模型，不作为外部输入/输出 contract 暴露。
- 外部网关集成域：`src/external_gateway/`
  - 承载 installer 对外部 edge gateway 的资产路由契约与产品策略，例如 `git-for-windows` release 到 gateway 候选的推断。
- 契约域：`src/contracts/`、`src/error.rs`、`src/installer_runtime_config.rs`
  - 负责外部输入/输出、退出码、环境变量和运行期配置边界。
- Shared foundation 依赖：`../omne_foundation/crates/http-kit/`
  - 提供通用 HTTP client、bounded body read / preview、bounded response streaming、URL 校验 / 脱敏、untrusted outbound policy 与 endpoint 探测。
- Shared foundation 依赖：`../omne_foundation/crates/github-kit/`
  - 提供 GitHub API 的纯 client 能力：repository 标识校验、latest release URL 构造、多 API base 回退，以及 release metadata DTO。
- Shared runtime 依赖：`../omne-runtime/crates/omne-artifact-install-primitives/`
  - 提供 artifact 候选下载执行、SHA 校验、direct binary 安装、archive binary 安装与 archive tree staging+replace 原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-integrity-primitives/`
  - 提供 `sha256` 解析、内容摘要计算与校验原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-host-info-primitives/`
  - 提供宿主 OS/arch 识别、canonical target triple 映射、target override 归一化、home 目录解析与目标可执行后缀原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-archive-primitives/`
  - 提供 archive/compression 格式识别、归档条目遍历和目标二进制提取原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-fs-primitives/`
  - 提供底层目录创建、暂存文件写入、权限设置、文件校验与原子替换原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-process-primitives/`
  - 提供宿主机命令探测、带输出捕获的命令执行、host recipe 执行、工作目录注入、默认 `sudo` 模式推断，以及命令路径解析 / 标准位置回退和 Unix 下对系统命令的 `sudo -n` 试探原语。
- Shared runtime 依赖：`../omne-runtime/crates/omne-system-package-primitives/`
  - 提供系统包管理器枚举、canonical 名称解析、安装 recipe 建模，以及按 OS 生成默认系统包安装配方的原语。
- 外部网关项目：`../toolchain-edge-gateway/`
  - 可选固定路由层，用于网络优化与反滥用；installer 只通过 `--gateway-base` 与其集成，不在本仓库内持有实现。

## 模块职责边界

- `application`
  - 面向对外用例编排，负责把 CLI / library 输入归一化成执行上下文，并调度具体领域策略。
  - 不持有内置工具安装规则、托管工具健康判定、plan method 字段矩阵或具体下载来源选择策略。
- `builtin_tools`
  - 承载“bootstrap 内置工具”领域策略，例如默认工具选择、宿主与托管安装健康探针、public release 资产选择，以及 git/gh/uv 的内置安装适配。
  - 不负责 CLI 解析、宿主/目标初始化、HTTP client 构建或结果总汇总。
- `install_plan`
  - 承载安装 plan 领域策略：plan method 归位、DTO -> 强类型条目收敛、方法分发和各 method 的安装执行策略。
  - 不负责执行上下文构建，也不直接承载顶层 use case 编排。
- `artifact`
  - 负责 installer 内部 artifact 安装结果模型，例如安装来源和可选 archive match 的归并。
  - 不属于外部 CLI/JSON contract，也不承载下载候选策略或 runtime 安装原语。
- `download_sources`
  - 负责 installer 自己的下载候选建模与 `gateway|canonical|mirror` 来源分类。
  - 不抽象成通用 HTTP foundation，也不承载 GitHub API client、artifact 落盘/解压安装或工具链布局策略。
- `external_gateway`
  - 负责 installer 对外部 edge gateway 的集成边界，包括固定资产路由拼装，以及 `git-for-windows` 这类产品特定 release URL/asset 到 gateway 候选的推断。
  - 不承载通用来源候选构造、GitHub API client 或下载/安装执行。
- `managed_toolchain`
  - 负责围绕 `managed_dir` 的托管工具链环境编排：收敛 `uv` 工具目录、Python 目录和缓存目录，解析托管根目录策略，并执行 `uv`、`uv python install`、`uv tool install`。
  - 对上层接收的是显式托管工具链方法分发，而不是在领域内部继续解析原始方法字符串或构建执行上下文。
  - 也拥有托管 `uv` 的 public release 供给能力，因为 `bootstrap` 与托管工具链执行都在复用同一套 `uv` 安装细节。
- `plan_items`
  - 负责 `install_plan` 与 `managed_toolchain` 共享的强类型安装项模型。
  - 只定义领域数据，不负责 plan 方法归一化、校验、下载候选或执行。
- `installer_runtime_config`
  - 负责把 `ExecutionRequest` / env 输入归一化成内部运行期策略对象。
  - 当前已经明确拆成 `github_releases`、`download_sources`、`download`、`package_indexes`、`python_mirrors`、`gateway` 六类策略，而不是让 GitHub API、镜像候选、索引、gateway、国家码和下载限制继续平铺混放。
- `omne-host-info-primitives`
  - 负责宿主 OS/arch 识别、canonical target triple 映射、target override 归一化、home 目录解析与目标可执行后缀推断。
  - 不负责 `OMNE_DATA_DIR`、`TOOLCHAIN_INSTALLER_MANAGED_DIR`、`managed_dir` 布局或 installer plan 语义。
- `http-kit`
  - 负责通用 HTTP client、bounded body read / preview、bounded response streaming、URL 校验 / 脱敏、untrusted outbound policy 与 HTTP 可达性探测。
  - 不承载 GitHub API schema、下载来源分类、镜像 / 网关候选顺序或安装器资产命名。
- `github-kit`
  - 负责 GitHub API 的纯 client 能力，例如 latest release metadata 获取。
  - 不负责环境变量读取、产品专属 user-agent、来源候选顺序、资产选择或安装执行。
- `omne-artifact-install-primitives`
  - 负责 artifact 候选下载执行、可选 SHA-256 校验，以及 direct binary / archive binary / archive tree 安装管道。
  - 不负责 GitHub release schema、来源候选顺序策略、产品目录布局或 installer 输出 contract。
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
  - 负责通用宿主机命令探测、带输出捕获的命令执行、host recipe 执行、工作目录注入、默认 `sudo` 模式推断、命令路径解析 / 标准位置回退，以及 Unix 下对 bare system command 的 `sudo -n` 试探。
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

## 刚性依赖方向

- `error`
  - 最低层；不允许依赖其他 installer 顶层模块。
- `contracts`
  - 只允许依赖 `error`。
- `application`
  - 可以依赖 `builtin_tools`、`contracts`、`error`、`install_plan`、`installer_runtime_config`、`managed_toolchain`。
  - 不反向提供给任何领域策略层依赖。
- `artifact`、`download_sources`、`installer_runtime_config`、`plan_items`
  - 只允许依赖更低层公共边界；不能反向依赖 `managed_toolchain`、`install_plan`、`builtin_tools`、`application`。
- `builtin_tools`
  - 可以依赖 `artifact`、`contracts`、`download_sources`、`error`、`external_gateway`、`installer_runtime_config`、`managed_toolchain`。
  - 不能依赖 `install_plan` 或 `application`。
- `external_gateway`
  - 只允许依赖 `installer_runtime_config`。
- `managed_toolchain`
  - 可以依赖 `artifact`、`contracts`、`download_sources`、`error`、`installer_runtime_config`、`plan_items`。
  - 不能反向依赖 `install_plan`、`builtin_tools` 或 `application`。
- `install_plan`
  - 可以依赖 `managed_toolchain` 及更低层模块，并负责执行编排。
  - 不能依赖 `builtin_tools` 或 `application`。
- 上述方向由 `tools/source-layout-check/` 在本地 git hook 与 CI 中共同执行。

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
