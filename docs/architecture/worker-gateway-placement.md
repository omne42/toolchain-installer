# Worker 网关归位

## 结论

当前这份 Cloudflare Worker 已拆分到独立项目 `../toolchain-edge-gateway/`。这是现在的正确归位：作为外部边缘部署适配器存在，而不是放进 `omne_foundation`、`omne-runtime`，也不继续留在 `toolchain-installer` 仓库内部。

`toolchain-installer` 现在只保留 `--gateway-base` 集成、来源候选参与规则和边界文档。

## 为什么不放进 shared runtime/foundation

- 它不是低层原语
  - 这份 Worker 不提供通用 HTTP、压缩、文件系统或进程执行能力。
  - 它承载的是具体部署策略：固定 GitHub release 路由、`CN` 国家限制、按 `IP + 路径` 限流。
- 它带有明显的供应商与部署耦合
  - 代码直接依赖 Cloudflare Worker 的请求模型、`cf-ipcountry`、`request.cf.country` 和边缘重定向语义。
  - 这类能力不适合作为跨平台运行时原语复用。
- CLI 不依赖它才能成立
  - 主安装流程在没有 Worker 的情况下也必须能通过官网或备用站完成。
  - 既然不是正确性前提，就不应被放进 installer 的共享底座层。
- 它表达的是产品策略，不是领域通用规则
  - 只允许 `git-for-windows` MinGit 路由，本质上是当前产品的流量治理策略。
  - 这和 `omne_foundation` 的通用网络能力、`omne-runtime` 的低层系统原语不是一个抽象层级。

## 当前放置

- 代码位置
  - `../toolchain-edge-gateway/`
- 角色
  - 可选 edge adapter
  - 为中国区提供固定路由优化
  - 把开放代理面收敛到白名单路径
- 文档位置
  - 网关项目内部负责自己的行为与安全文档
  - installer 仓库只保留集成边界与来源选择规则

## 什么时候才该拆出去

下面这些条件已经足够支持独立拆分：

- 部署形态与 Rust CLI 完全不同。
- 生命周期和验证命令与 installer 不同。
- 行为属于边缘策略，而不是安装域原语。
- 后续公网暴露、集中式限流和部署说明会继续独立演进。

因此现在直接拆成独立项目，比继续挂在 installer 仓库里更清晰。

## 明确的非目标归位

- 不放进 `../omne_foundation/crates/http-kit/`
  - `http-kit` 负责通用 HTTP client、bounded body read / preview、URL 校验 / 脱敏、untrusted outbound policy 和连通性探测，不负责产品级边缘限流、国家策略，也不承载 release 元数据或来源候选策略。
- 不放进 `../omne-runtime/`
  - runtime 负责文件、进程、完整性、archive 等宿主机原语，不负责 Cloudflare 专属网关策略。
- 不并入主 CLI Rust 代码
  - CLI 只负责来源选择与安装编排，不内嵌 Cloudflare Worker 的部署逻辑。

## 与主 CLI 的边界

- CLI 可以使用 `gateway-base` 这类配置把 Worker 视为一个可选来源候选。
- Worker 不参与 plan 解释、二进制解压、摘要校验或最终落盘。
- Worker 不可成为唯一下载入口；官网和备用站仍然必须可直接工作。

## 继续阅读

- `system-boundaries.md`
- `../operations/external-gateway-integration.md`
- `../references/source-selection-rules.md`
