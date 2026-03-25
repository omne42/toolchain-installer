# 交付路线图

## 当前已落地能力

- Rust CLI 与库入口已经分离，`main.rs` 只做进程入口。
- 安装能力已按领域拆分为 `bootstrap`、`plan`、`platform`、`installation`、`uv`；共享 HTTP foundation/runtime 原语直接回归 `../omne_foundation/` 与 `../omne-runtime/`，而 GitHub release 元数据与下载来源策略保留在 installer 自身。CI 通过 `.github/actions/checkout-shared-deps/` 复用 sibling checkout 链路后，仍可完整构建并执行安装验证。
- `release`、`system_package|apt`、`pip`、`uv`、`uv_python`、`uv_tool` 六类安装基建已纳入统一 plan 执行面。
- `examples/python-plan.json` 已覆盖 `uv + Python 3.13.12 + ruff + mypy` 的组合安装。
- 外部边缘网关已拆分到 `../toolchain-edge-gateway/`，installer 仓库只保留集成边界与来源规则。

## 下一阶段

1. 多平台验证深化
   - 继续扩大 Linux、macOS、Windows 上的真实安装验证覆盖，尤其是 `uv_python` 与 `uv_tool` 的来源回退路径。
2. 外部网关硬化
   - 在独立网关项目中补齐公网暴露场景下的集中式限流与部署说明。
3. 调用方集成沉淀
   - 补充调用方接入指南，避免各调用方重复封装安装细节。
4. 文档机械化维护
   - 继续增加链接校验、陈旧文档探测与执行计划归档规范。

## 交接最小信息

- 契约入口：`../contracts/cli-surface.md`
- plan 约束：`../contracts/install-plan-contract.md`
- 架构边界：`../architecture/system-boundaries.md`
- 安全边界：`../operations/security-boundaries.md`
- 文档系统入口：`../docs-system-map.md`

## 交付原则

- 先守住契约与边界，再扩展安装方法。
- 计划、路线图和技术债都留在仓库内，不依赖仓外上下文。
- 新功能优先补充可执行示例与验证，再补外围说明。
