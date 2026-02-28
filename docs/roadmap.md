# 路线图与交接清单

## 最终交付形态

- 一个单独仓库维护的安装器 CLI（通用复用）。
- 一个可选 Worker 路由实现（固定白名单 + 反滥用）。
- `omne-agent` 通过稳定契约调用安装器，不再内置复杂下载逻辑。

## 分阶段计划

## 阶段 1：基础工程

- 初始化 Rust 项目与模块骨架。
- 实现 `bootstrap` 命令框架与 JSON 输出框架。
- 完成基础单测框架。

验收：
- `cargo check`、`cargo test` 通过。

## 阶段 2：下载与安装核心

- 实现平台识别与资产解析。
- 实现下载候选与镜像回退。
- 实现完整性校验与原子安装。

验收：
- mock 源端到端测试通过（成功/失败路径都覆盖）。

## 阶段 3：网关与安全

- 实现 Worker 固定路由。
- 实现白名单与拒绝策略。
- 完成限流策略与对应测试。

验收：
- 非法路由拒绝测试通过；合法路由重定向测试通过。

## 阶段 4：调用方接入

- 在 `omne-agent` 接入外部安装器。
- 保持 `omne toolchain bootstrap --json` 字段兼容。
- 增加回归测试与边界扫描。

验收：
- `omne-agent` 相关测试通过；
- `crates/app-server` 无安装实现。

## 当前执行顺序（严格）

1. 文档完成并逐篇提交。  
2. 安装器 CLI 实现与测试。  
3. Worker 实现与测试。  
4. `omne-agent` 接入与全链路验证。  
5. SSH 推送与最终验收报告。  

## 交接最小信息

- 当前分支：`feat/bootstrap-installer-foundation`
- 必跑命令：`cargo check`、`cargo test`
- 与调用方契约：`docs/contract.md`
- 安全边界：`docs/security.md`
