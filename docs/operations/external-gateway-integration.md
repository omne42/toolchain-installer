# 外部网关集成

## 角色

`toolchain-installer` 不再在仓库内持有 Cloudflare Worker 实现。可选固定路由网关已拆分到同工作区的独立项目 `../toolchain-edge-gateway/`。

installer 只负责把外部网关视为一个可选来源候选，而不是安装正确性的唯一前提。

## 集成边界

- 代码归位
  - gateway 路由与 `git-for-windows` 候选推断位于 `src/external_gateway/asset_routing.rs`。
  - 通用下载候选构造仍位于 `src/download_sources.rs`。
- CLI 参数
  - `--gateway-base <url>` 用于注入某个已部署网关实例的基地址。
  - `--country <ISO2>` 用于声明当前国家码；也可由 `TOOLCHAIN_INSTALLER_COUNTRY` 提供。
  - `TOOLCHAIN_INSTALLER_GATEWAY_BASE` 可在未显式传参时提供同一基地址。
- 参与条件
  - 当前只有 `country=CN` 且目标 URL 精确匹配 `https://github.com/git-for-windows/git/releases/download/<tag>/<asset>` 这类 canonical `git-for-windows` release 资产时，才会生成网关候选；只要附带 query、fragment，或 `<tag>` / `<asset>` 不是单一路径段，就会直接拒绝 gateway 候选。
  - 若只提供 `gateway-base` 而没有国家码，不会强行启用网关候选。
  - 自定义 mirror URL、代理 URL，或仅仅在 query/path 里“包含那段字符串”的普通下载地址，都不会被静默改写成网关候选。
- 回退规则
  - 网关不可达、拒绝或未命中时，仍然必须继续尝试官网或备用站。
- 正确性边界
  - installer 仍负责下载、摘要校验、解压、权限设置和最终落盘。
  - 网关不参与 plan 解释、安装方法选择或文件写入。

## 不在本仓库定义的事实

下面这些事实不再由 `toolchain-installer` 仓库作为唯一来源维护：

- Worker 的 Cloudflare 运行时实现细节
- 固定路由白名单
- 国家限制与限流阈值
- 网关自身的本地测试命令

这些事实的当前记录位置：

- `../../../toolchain-edge-gateway/docs/architecture/system-boundaries.md`
- `../../../toolchain-edge-gateway/docs/operations/request-routing-and-rate-limits.md`
- `../../../toolchain-edge-gateway/docs/operations/security-boundaries.md`

## 调用方约束

- 不要把外部网关当成唯一下载入口。
- 不要假定所有工具都会经过 `gateway-base`。
- 如果调用方关心审计，应记录传入的 `gateway-base`、国家码和最终命中的来源种类。

## 本仓库何时需要同步更新

- `--gateway-base` 的触发条件变化。
- 网关候选参与下载来源排序的规则变化。
- installer 与外部网关的职责边界变化。
