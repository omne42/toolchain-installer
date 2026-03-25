# 来源选择规则

## 总原则

- 优先官方来源。
- 备用站只作为可达性和网络条件不佳时的回退。
- 回退顺序由调用方显式追加的来源与内置来源共同决定。
- 可选 Worker 只做固定路由优化，不做开放代理。

## `release` 方法

- 基于工具、版本、平台资产匹配规则生成候选。
- `--mirror-prefix` 可以追加候选前缀。
- `country=CN` 且目标满足 `git release` 条件时，可通过 `gateway-base` 走固定网关。
- `gateway-base` 指向的是外部网关部署实例，而不是 installer 仓库内建服务。

## `uv_tool` 方法

- 官方 PyPI `https://pypi.org/simple` 隐式存在。
- `--package-index` 与 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES` 追加备用索引。
- 安装前先做可达性探测，再按可达结果选择索引。

## `uv_python` 方法

- 官方 Python 下载来源先尝试。
- `--python-mirror` 与 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS` 追加备用镜像。
- 当前宿主环境内的可达性结果决定最终使用哪个来源。

## 网关边界

- 只允许固定白名单路由。
- 不允许任意查询参数。
- 不代理 `gh` 或其他非白名单工具。
- 默认用重定向而不是代传大文件。

## 调用方要求

- 如果调用方关心审计与回退行为，应记录传入的备用索引与镜像顺序。
- 不要把备用网址写死在多个地方；优先通过 CLI 参数或环境变量集中注入。
