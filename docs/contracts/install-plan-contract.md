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
- 顶层对象和每个 `items[]` 对象都启用严格未知字段校验；拼错字段名会在反序列化阶段直接失败，不会被静默吞掉。
- `plan.items[*].id` 必须全局唯一；重复 `id` 会在执行前返回退出码 `2`。
- `method` 必须是受支持的方法名；未知方法会在执行前直接返回退出码 `2`。
- 不属于该方法的字段组合会在执行前返回退出码 `2`，不会静默忽略。
- 纯库 API `validate_install_plan()` 只做 schema、字段组合、宿主/目标约束和重复 `id` 校验；它不知道 `managed_dir`，因此不会擅自猜测依赖目标目录的全局路径冲突。
- CLI 与 `validate_install_plan_with_request()` 会在结构校验通过后，再结合真实 `managed_dir` 做全局目标路径冲突校验；若解析后的目标路径发生冲突，会在执行前返回退出码 `2`，不会依赖执行顺序“碰巧覆盖”。
- `src/contracts/install_plan_contract.rs` 只承载外部 JSON DTO；进入 `src/install_plan/` 后会先收敛成内部强类型 `ResolvedPlanItem`，执行层不再直接处理一组弱类型 `Option<String>` 字段。

## 方法清单

- `release`
  - 下载 release 资产并安装到目标路径。
- `archive_tree_release`
  - 下载 archive 资产并把完整目录树解到目标路径。
- `system_package`
  - 通过宿主系统包管理器安装。
- `apt`
  - 显式通过 canonical `apt-get` 安装。
- `pip`
  - 通过 `python -m pip install` 安装。
- `npm_global`
  - 通过 `npm`、`pnpm` 或 `bun` 安装宿主机全局 JS CLI。
- `workspace_package`
  - 在目标工作区目录里安装前端或 Node workspace 依赖。
- `cargo_install`
  - 通过 `cargo install` 安装 Rust CLI。
- `rustup_component`
  - 通过 `rustup component add` 安装 Rust 官方组件。
- `go_install`
  - 通过 `go install` 安装 Go CLI。
- `uv`
  - 下载并安装宿主平台对应的官方 `uv` 独立二进制。
- `uv_python`
  - 通过 `uv python install` 安装指定 Python 版本，并把可执行文件落到 `managed_dir`。
- `uv_tool`
  - 通过 `uv tool install` 安装 `ruff`、`mypy` 等工具，并把可执行文件落到 `managed_dir`。

## 方法归位

- `release`
  - `archive_tree_release`
  - 归属于 release 安装域。
- `system_package`、`apt`
  - 归属于宿主系统包安装域。
- `pip`
  - 归属于 Python 包安装域。
- `npm_global`
  - 归属于 Node / JS 全局工具安装域。
- `workspace_package`
  - 归属于工作区依赖安装域。
- `cargo_install`、`rustup_component`
  - 归属于 Rust 宿主工具安装域。
- `go_install`
  - 归属于 Go 宿主工具安装域。
- `uv`、`uv_python`、`uv_tool`
  - 归属于托管工具链环境域。
  - 执行前会先把原始 `method` 字符串归位成显式托管工具链方法，再进入对应领域分发。

## 字段矩阵

- `release`
  - 允许 `url`、`sha256`、`archive_binary`、`binary_name`、`destination`。
- `archive_tree_release`
  - 允许 `url`、`sha256`、`destination`。
- `system_package`
  - 允许 `package`、可选 `manager`。
- `apt`
  - 允许 `package`、可选 `manager=apt-get`。
- `pip`
  - 允许 `package`、可选 `python`。
- `npm_global`
  - 允许 `package`、可选 `manager=npm|pnpm|bun`、可选 `binary_name`。
- `workspace_package`
  - 允许 `package`、`destination`、可选 `manager=npm|pnpm|bun`。
- `cargo_install`
  - 允许 `package`、可选 `version`、可选 `binary_name`。
- `rustup_component`
  - 允许 `package`、可选 `binary_name`。
- `go_install`
  - 允许 `package`、可选 `version`、可选 `binary_name`。
- `uv`
  - 不接受额外字段。
- `uv_python`
  - 允许 `version`。
- `uv_tool`
  - 允许 `package`、可选 `python`、可选 `binary_name`。

## 宿主与目标约束

- `bootstrap` 仅支持当前宿主机，即 `target_triple` 必须等于自动探测到的 `host_triple`。
- `method=release` 支持显式跨目标平台下载与落盘。
- `method=archive_tree_release` 支持显式跨目标平台下载与解包。
- `method=system_package|apt|pip|npm_global|workspace_package|cargo_install|rustup_component|go_install|uv|uv_python|uv_tool` 仅作用于当前宿主机。
- 若宿主机方法出现 `target_triple != host_triple`，执行前直接返回退出码 `2`。

## 路径与 URL 约束

- `release.url` 仅允许 `http` 或 `https`。
- `archive_tree_release.url` 仅允许 `http` 或 `https`，且资产名必须是受支持的 `.tar.gz`、`.tar.xz` 或 `.zip`。
- `destination` 若为相对路径，会解析到 `managed_dir` 下。
- Unix 风格绝对路径如 `/tmp/demo` 会被拒绝，避免绕过托管目录边界。
- 任意 `destination` 都禁止包含 `..`，避免路径逃逸。
- 为了避免 Windows 语义下的伪相对路径逃逸，`destination` 还禁止使用 `C:foo` 这类 drive-relative 路径，以及 `\foo` 这类 root-relative 路径。
- Windows 绝对路径如 `C:\tools\demo.exe` 会按绝对路径处理，不会被错误拼到 `managed_dir` 下。
- `release` 未指定 `destination` 时，默认安装到 `managed_dir/<binary_name>`。
- `archive_tree_release` 未指定 `destination` 时，默认解到 `managed_dir/<id>/`。
- `archive_tree_release` 会先把 archive 解到同级 staging 目录，只有校验和解包都成功后才替换目标目录；失败时不会先删除现有内容。
- `workspace_package` 必须显式给出 `destination`，不会默认写入 `managed_dir`。
- `npm_global`、`cargo_install`、`go_install` 的最终可执行文件路径以结果里的 `destination` 为准；调用方不应假设它们都严格等于 `managed_dir/<binary>`。
- `uv_tool` 若提供 `binary_name`，结果里的 `destination` 会指向该二进制在 `managed_dir` 下的实际路径；安装成功后若该路径不存在，整项返回失败。
- 所有可确定最终输出路径的方法都参与全局冲突校验；两个 item 不能指向同一个目标路径。
- `uv_python` 会占用 `managed_dir/.uv-python` 这块托管安装根，因此它也参与与该路径相关的冲突校验。
- `uv_python` 只有在 `managed_dir` 下实际发现匹配版本的 Python 可执行文件后才算成功；单纯 `uv python install` 退出码为 `0` 不构成成功条件。

## 来源探测与回退

- `uv_tool`
  - 当调用方没有显式提供任何包索引时，默认只使用官方 PyPI `https://pypi.org/simple`。
  - 当调用方显式提供了 `--package-index` 或 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES` 时，installer 不再隐式把官方 PyPI 插到最前面；显式索引顺序就是候选顺序。
  - 若显式索引、镜像或镜像前缀里出现重复值，只会保留第一次出现的位置，不会按字典序重排。
  - 安装前会探测显式索引的可达性，把可达源优先用于安装。
- `uv_python`
  - 会先尝试官方 Python 下载来源，失败后再按顺序回退到 `--python-mirror` 或 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS` 提供的备用站。
  - 备用镜像列表若有重复值，只保留第一次出现的位置。
- `release`、`archive_tree_release`
  - 资产类型判断基于 URL 的 path 最后一段，不把 query string 当成资产名的一部分；`tool.tar.gz?download=1` 仍按 `tool.tar.gz` 处理。
- `release`
  - 通过内置来源规则、镜像前缀与可达性结果确定下载候选顺序。
  - `--mirror-prefix` 与 `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES` 的重复值只去重，不重排显式顺序。

## 参考输入

- Python 工具链组合 plan：`../../examples/python-plan.json`
- 单独安装 `uv`：`../../examples/uv-plan.json`
- 单独安装 `ruff`：`../../examples/ruff-plan.json`
- 单独安装 `mypy`：`../../examples/mypy-plan.json`

这些文件是可执行参考，不应在文档里维护第二套不同版本的示例。
