# 文档系统地图

## 目标

这个仓库把 `docs/` 视为记录系统，而不是零散说明集合。入口文档保持短小，深层事实放在职责明确的文件里，通过交叉链接逐步展开。

## 入口分工

- `README.md`：给仓库访客的外部概览，只回答“这个仓库是什么、去哪里继续看”。
- `AGENTS.md`：给执行者的短地图，只指路，不堆积细节。
- `docs/`：事实来源，按领域、职责、边界拆分。
- `examples/*.json`：可执行参考输入，不在文档里重复维护第二份 schema。

## 目录职责

- `docs/architecture/`
  - `system-boundaries.md`：系统边界、宿主/目标语义、模块交互。
  - `source-layout.md`：源码树与文件职责。
  - `worker-gateway-placement.md`：可选 Cloudflare Worker 应归位在哪一层，以及何时才值得拆成独立网关项目。
- `docs/contracts/`
  - `cli-surface.md`：CLI 命令、参数、输出与退出码。
  - `install-plan-contract.md`：plan schema、方法矩阵、字段约束。
- `docs/guides/`
  - `installation-examples.md`：跨语言安装示例与入口命令。
  - `python-toolchain-bootstrap.md`：Python 3.13.12 + uv + ruff + mypy 的实际引导。
- `docs/operations/`
  - `security-boundaries.md`：威胁模型、白名单、反滥用策略。
  - `external-gateway-integration.md`：installer 如何集成外部固定路由网关，以及哪些事实已迁到独立网关项目。
  - `quality-and-doc-maintenance.md`：质量门槛、验证命令、文档更新规则。
- `docs/plans/`
  - `delivery-roadmap.md`：当前交付状态与后续里程碑。
  - `documentation-tech-debt.md`：文档系统与知识库相关债务。
- `docs/references/`
  - `example-plan-files.md`：`examples/*.json` 的职责索引。
  - `source-selection-rules.md`：官网、镜像、网关与回退顺序规则。

## 命名规则

- 文件名必须直接暴露职责，避免无法从路径推断内容的通用名字。
- 目录负责收纳领域，文件负责表达单一主题。
- 变更优先更新已有文档；只有当单个文件同时承担多个主题时再拆分。

## 新鲜度规则

- 行为变化与文档变化同 PR 提交。
- 新模块、新示例、新契约字段必须补充对应索引文档。
- 过时路径应删除或迁移，避免同一事实存在两份版本。
- 计划、路线图、技术债务都是仓库内版本化产物，不依赖外部聊天上下文。

## 最小导航顺序

1. 先看 `README.md` 了解目标与入口。
2. 需要执行修改时看 `AGENTS.md`。
3. 涉及结构时读 `docs/architecture/`。
4. 涉及 CLI 或 plan 时读 `docs/contracts/`。
5. 需要命令示例时读 `docs/guides/` 与 `docs/references/`。
6. 需要判断风险与完成标准时读 `docs/operations/` 与 `docs/plans/`。
