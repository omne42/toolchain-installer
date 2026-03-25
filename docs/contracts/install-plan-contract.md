# 安装 Plan 契约

## 目标

plan 模式让调用方声明“装什么”，安装器只提供执行基建，不把安装策略反向耦合进调用方仓库。

## 顶层结构

```json
{
  "schema_version": 1,
  "items": [
    {
      "id": "uv",
      "method": "uv"
    }
  ]
}
```

规则：

- `schema_version` 当前固定为 `1`。
- `plan.items` 不能为空。
- 不属于该方法的字段组合会在执行前返回退出码 `2`，不会静默忽略。

## 方法清单

- `release`
  - 下载 release 资产并安装到目标路径。
- `system_package`
  - 通过宿主系统包管理器安装。
- `apt`
  - 显式通过 `apt` 或 `apt-get` 安装。
- `pip`
  - 通过 `python -m pip install` 安装。
- `uv`
  - 下载并安装宿主平台对应的官方 `uv` 独立二进制。
- `uv_python`
  - 通过 `uv python install` 安装指定 Python 版本，并把可执行文件落到 `managed_dir`。
- `uv_tool`
  - 通过 `uv tool install` 安装 `ruff`、`mypy` 等工具，并把可执行文件落到 `managed_dir`。

## 方法归位

- `release`
  - 归属于 release 安装域。
- `system_package`、`apt`
  - 归属于宿主系统包安装域。
- `pip`
  - 归属于 Python 包安装域。
- `uv`、`uv_python`、`uv_tool`
  - 归属于托管工具链环境域。
  - 执行前会先把原始 `method` 字符串归位成显式托管工具链方法，再进入对应领域分发。

## 字段矩阵

- `release`
  - 允许 `url`、`sha256`、`archive_binary`、`binary_name`、`destination`。
- `system_package`
  - 允许 `package`、可选 `manager`。
- `apt`
  - 允许 `package`、可选 `manager=apt|apt-get`。
- `pip`
  - 允许 `package`、可选 `python`。
- `uv`
  - 不接受额外字段。
- `uv_python`
  - 允许 `version`。
- `uv_tool`
  - 允许 `package`、可选 `python`。

## 宿主与目标约束

- `bootstrap` 仅支持当前宿主机，即 `target_triple` 必须等于自动探测到的 `host_triple`。
- `method=release` 支持显式跨目标平台下载与落盘。
- `method=system_package|apt|pip|uv|uv_python|uv_tool` 仅作用于当前宿主机。
- 若宿主机方法出现 `target_triple != host_triple`，执行前直接返回退出码 `2`。

## 路径与 URL 约束

- `release.url` 仅允许 `http` 或 `https`。
- `destination` 若为相对路径，会解析到 `managed_dir` 下。
- 任意 `destination` 都禁止包含 `..`，避免路径逃逸。
- `release` 未指定 `destination` 时，默认安装到 `managed_dir/<binary_name>`。

## 来源探测与回退

- `uv_tool`
  - 会先探测官方 PyPI 与备用索引的可达性，把可达源优先用于安装。
- `uv_python`
  - 会先尝试官方 Python 下载来源，失败后再按顺序回退到 `--python-mirror` 或 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS` 提供的备用站。
- `release`
  - 通过内置来源规则、镜像前缀与可达性结果确定下载候选顺序。

## 参考输入

- Python 工具链组合 plan：`../../examples/python-plan.json`
- 单独安装 `uv`：`../../examples/uv-plan.json`
- 单独安装 `ruff`：`../../examples/ruff-plan.json`
- 单独安装 `mypy`：`../../examples/mypy-plan.json`

这些文件是可执行参考，不应在文档里维护第二套不同版本的示例。
