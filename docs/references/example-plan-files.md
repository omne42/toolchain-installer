# 示例 Plan 文件索引

## 目的

`examples/*.json` 是可执行参考输入。它们既是示例，也是调用方可以直接复用的最小 plan 样本。

## 文件职责

- `../../examples/uv-plan.json`
  - 仅安装 `uv`。
- `../../examples/python-plan.json`
  - 安装 `uv`、Python `3.13.12`、`ruff`、`mypy`。
- `../../examples/ruff-plan.json`
  - 仅安装 `ruff`，并绑定 Python `3.13.12`。
- `../../examples/mypy-plan.json`
  - 仅安装 `mypy`，并绑定 Python `3.13.12`。
- `../../examples/nodejs-plan.json`
  - Node.js release 安装示例。
- `../../examples/go-plan.json`
  - Go release 安装示例。
- `../../examples/rust-plan.json`
  - Rust 工具链引导示例。

## 使用规则

- 修改 plan schema 时，同时检查这些示例是否仍然可执行。
- 新增示例 plan 时，把职责补到本文件。
- 文档中的命令示例优先引用这里列出的文件，减少重复维护。
