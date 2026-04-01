# 来源选择规则

## 总原则

- 优先官方来源。
- 备用站只作为可达性和网络条件不佳时的回退。
- 回退顺序由调用方显式追加的来源与内置来源共同决定。
- 可选 Worker 只做固定路由优化，不做开放代理。

## `release` 方法

- 基于工具、版本、平台资产匹配规则生成候选。
- 路径落点继续遵循 plan contract：Windows 绝对 `destination` 会保留为绝对路径，不会被拼到 `managed_dir` 下。
- `--mirror-prefix` 与 `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES` 可以追加候选前缀。
- 显式提供的 mirror 前缀会按传入顺序去重，不会按字典序重排。
- `TOOLCHAIN_INSTALLER_GITHUB_API_BASES` 可覆盖 release metadata API base；未设置时默认只使用官方 `https://api.github.com`。
- `country=CN` 且目标满足 `git release` 条件时，可通过 `gateway-base` 走固定网关。
- `--gateway-base` / `TOOLCHAIN_INSTALLER_GATEWAY_BASE` 与 `--country` / `TOOLCHAIN_INSTALLER_COUNTRY` 共同决定是否生成网关候选。
- `gateway-base` 指向的是外部网关部署实例，而不是 installer 仓库内建服务。

## `uv_tool` 方法

- 当调用方没有显式提供任何索引时，默认只使用官方 PyPI `https://pypi.org/simple`。
- `--package-index` 与 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES` 一旦提供，就由这些显式索引定义候选顺序；installer 不再把官方 PyPI 隐式插到最前面。
- 显式索引会按调用方给出的顺序去重，不会被内部集合重排。
- 复用托管 `uv` 之前会先做带超时上限的 `uv --version` 健康探测；探测失败或超时都会触发重装，避免坏掉或挂起的 `uv` 卡住后续安装。
- 安装前先做可达性探测，再按可达结果优先尝试显式索引。
- 对外 JSON 结果里的 `source` 会脱敏显式索引 URL，不回显用户信息、query 或 fragment。
- 宿主 shell 已存在的 `UV_*` 环境变量不会透传到托管 `uv tool install`；来源选择只由 installer 自己的托管环境和显式索引配置驱动。

## `uv_python` 方法

- 官方 Python 下载来源先尝试。
- `--python-mirror` 与 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS` 追加备用镜像。
- 显式 Python mirror 会按传入顺序去重，回退顺序与调用方声明保持一致。
- 当前宿主环境内的可达性结果决定最终使用哪个来源。
- 复用托管 `uv` 之前会先做带超时上限的 `uv --version` 健康探测；探测失败或超时都会触发重装，避免坏掉或挂起的 `uv` 卡住后续安装。
- 对外 JSON 结果里，官方来源会标成 `source_kind=canonical`，显式镜像才会标成 `python_mirror`。
- 对外 JSON 结果里的 `source` 会脱敏显式镜像 URL，不回显用户信息、query 或 fragment。
- 宿主 shell 已存在的 `UV_*` 环境变量不会透传到托管 `uv python install`；来源选择只由 installer 自己的托管环境和显式 mirror 配置驱动。

## 网关边界

- 只允许固定白名单路由。
- 不允许任意查询参数。
- 不代理 `gh` 或其他非白名单工具。
- 默认用重定向而不是代传大文件。

## 调用方要求

- 如果调用方关心审计与回退行为，应记录传入的备用索引与镜像顺序。
- 不要把备用网址写死在多个地方；优先通过 CLI 参数或环境变量集中注入。
