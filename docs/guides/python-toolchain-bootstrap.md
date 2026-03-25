# Python 3.13.12 工具链引导

## 目标

在当前宿主平台上自动完成以下步骤：

1. 安装 `uv`
2. 安装 Python `3.13.12`
3. 安装并绑定 `ruff`
4. 安装并绑定 `mypy`
5. 对官网与备用站做可达性探测后再选择实际下载源

这里的“不同平台”语义是：在 Linux、macOS、Windows 等不同宿主平台执行时，安装器都会在各自运行时环境内探测可达源，而不是在单次执行里替所有平台做远程测试。

## 推荐执行方式

```bash
toolchain-installer bootstrap --json \
  --plan-file examples/python-plan.json \
  --package-index "https://pypi.tuna.tsinghua.edu.cn/simple" \
  --python-mirror "https://mirror.example/python-build-standalone"
```

对应 plan 文件：`../../examples/python-plan.json`

如果不显式传 `--managed-dir`，`uv`、Python、`ruff`、`mypy` 默认会托管到 `~/.omne_data/toolchain/<target>/bin` 及其相邻隐藏子目录，而不是系统级位置。

## plan 内容

`examples/python-plan.json` 依次执行：

- `uv`
- `uv_python` with `version=3.13.12`
- `uv_tool` with `package=ruff` and `python=3.13.12`
- `uv_tool` with `package=mypy` and `python=3.13.12`

## 来源选择规则

- `uv_tool`
  - 官方 PyPI `https://pypi.org/simple` 会隐式参与候选。
  - `--package-index` 追加的是备用索引，安装器会先做可达性探测，再优先使用可达源。
- `uv_python`
  - 官方 Python 下载来源会先尝试。
  - `--python-mirror` 与 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS` 提供顺序回退的备用站。
- 所有探测都发生在当前宿主环境，因此不同平台会得到各自独立的可达性结果。

## 直接参数模式

安装 Python `3.13.12`：

```bash
toolchain-installer bootstrap --json \
  --method uv_python \
  --id python3.13.12 \
  --tool-version 3.13.12 \
  --python-mirror "https://mirror.example/python-build-standalone"
```

安装 `ruff`：

```bash
toolchain-installer bootstrap --json \
  --method uv_tool \
  --id ruff \
  --package ruff \
  --python 3.13.12 \
  --package-index "https://pypi.tuna.tsinghua.edu.cn/simple"
```

安装 `mypy`：

```bash
toolchain-installer bootstrap --json \
  --method uv_tool \
  --id mypy \
  --package mypy \
  --python 3.13.12 \
  --package-index "https://pypi.tuna.tsinghua.edu.cn/simple"
```

## 宿主约束

- `uv`、`uv_python`、`uv_tool` 都是宿主机方法。
- 这些方法要求 `target_triple == host_triple`。
- 如果调用方要为其他平台预置二进制，应改用 `release` 方法，而不是跨目标执行 `uv` 相关方法。

## 继续阅读

- 方法与字段矩阵：`../contracts/install-plan-contract.md`
- 来源优先级：`../references/source-selection-rules.md`
