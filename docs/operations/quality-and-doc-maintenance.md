# 质量与文档维护

## 质量门槛

每次变更至少应通过：

- `cargo fmt --all`
- `cargo check --all-targets`
- `cargo test --all-targets`
- 若改动安装链路、平台分支或 workflow 验证，额外运行 `cargo build --release` 与 `python3 scripts/install_smoke.py --binary ./target/release/toolchain-installer --phase ...`
- 若改动外部网关项目或 installer 与网关的边界，再运行 `cd ../toolchain-edge-gateway && npm test`

其中 `cargo test --all-targets` 会覆盖：

- 库内单元测试
- CLI 端到端测试
- `tests/docs_system.rs` 的文档系统结构检查

CI 另外会在 GitHub-hosted Linux、macOS、Windows runner 上调用 `scripts/install_smoke.py`，执行真实网络下载和真实宿主安装 smoke。当前覆盖这些安装面：

- `bootstrap --tool gh`
- Windows 上的 `bootstrap --tool git`
- `plan.method=release`
- `plan.method=system_package`
- Ubuntu 上的 `plan.method=apt`
  - CI 会单独跑公开 alias `apt`，而不是只通过 `system_package + manager=apt-get` 间接覆盖。
- `plan.method=pip`
- `plan.method=uv`
- `plan.method=uv_python`
- `plan.method=uv_tool`

## 文档维护规则

- `README.md`
  - 保持短小，只做仓库概览与入口导航。
- `AGENTS.md`
  - 保持短小，只做执行者地图，不堆积长篇约束。
- `docs/`
  - 作为记录系统保存稳定事实。
- `examples/*.json`
  - 作为可执行参考输入，和文档一起维护。

## Git Hooks

- 仓库提供 `.githooks/pre-commit`，用于检查 `docs/architecture/source-layout.md` 是否与当前 `src/` 的真实源码树双向一致，并校验顶层模块依赖方向。
- 安装方式：
  - `git config core.hooksPath .githooks`
- hook 实际调用：
  - `cargo run --quiet --locked --manifest-path tools/source-layout-check/Cargo.toml -- --staged`
- `--staged` 语义意味着 hook 校验的是即将提交的 index 快照，不是未暂存的工作树；源码增删或 `source-layout.md` 更新都必须一起 `git add`。
- `source-layout.md` 中所有顶层 `src/*/` 目录条目必须按字母序排列，不能按主观重要性或语义分组打乱顺序。
- 检查失败时，hook 会显性披露缺失的 `src/...` 文件路径、文档中残留的失效 `src/...` 条目、顶层目录集合不一致、目录顺序错误，以及违反架构方向的模块依赖，而不是静默放过结构漂移。
- 新增任何顶层模块时，除了更新 `docs/architecture/source-layout.md`，还必须同步更新检查器中的显式依赖方向策略；未声明策略的模块会直接阻止提交。
- Rust 检查器源码位于 `tools/source-layout-check/`；它不依赖 installer 主 crate，承担源码布局与顶层依赖方向的一致性校验。

## 同步更新规则

- CLI 参数、输出字段或退出码变化时，更新 `docs/contracts/cli-surface.md`。
- plan 方法、字段矩阵或宿主/目标规则变化时，更新 `docs/contracts/install-plan-contract.md`。
- 来源优先级、官网/备用站探测或网关策略变化时，更新 `docs/references/source-selection-rules.md`。
- installer 对外部网关的集成边界变化时，更新 `docs/operations/external-gateway-integration.md`。
- Worker 的归位、拆分策略或和 shared runtime/foundation 的边界变化时，更新 `docs/architecture/worker-gateway-placement.md`。
- 源码目录调整时，更新 `docs/architecture/source-layout.md`。
- 新示例 plan 落仓时，更新 `docs/references/example-plan-files.md`。

## 文档写作约束

- 优先改现有事实文件，不把新增知识留在聊天里。
- 文件名必须表达职责，避免再回到扁平且模糊的文档命名。
- 一份事实只保留一处系统级定义，其他位置通过链接引用。
- 文档与代码冲突时，以代码行为为准，并在同一改动中修正文档。

## 当前缺口

已知仍需继续补强的项记录在 `../plans/documentation-tech-debt.md`。
