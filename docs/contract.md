# CLI 契约

## 命令

`toolchain-installer bootstrap [options]`

## 输入参数

- `--tool <name>`：可重复，默认 `git` 和 `gh`。
- `--target-triple <triple>`：可选，覆盖自动探测目标平台。
- `--managed-dir <path>`：可选，安装输出目录。
- `--mirror-prefix <prefix>`：可重复，追加下载候选前缀。
- `--gateway-base <url>`：可选，固定网关入口。
- `--country <ISO2>`：可选，调用方传入地区码（如 `CN`）。
- `--plan-file <path>`：可选，执行调用方提供的安装计划（JSON）。
- `--method <release|system_package|apt|pip>`：可选，执行单个安装项（免 JSON）。
- `--id <name>`：可选，单个安装项标识（与 `--method` 配套）。
- `--url` / `--sha256` / `--archive-binary` / `--binary-name`：`release` 模式参数。
- `--package` / `--manager`：`system_package` 或 `apt` 模式参数。
- `--python`：`pip` 模式指定 Python 解释器（默认 `python3`）。
- `--json`：输出机器可读 JSON。
- `--strict`：任一工具失败时返回非 0。

## Plan 模式

调用方可通过 `--plan-file` 提供安装计划，安装器只提供基建执行，不决定装什么。

- `method=release`：下载 release 资产并安装到目标路径。
- `method=system_package|apt`：通过系统包管理器安装。
- `method=pip`：通过 `python -m pip install` 安装。

## 直接参数模式

调用方可不写 JSON，直接传 `--method` 与对应参数执行单个安装项。

## JSON 输出

```json
{
  "schema_version": 1,
  "target_triple": "x86_64-unknown-linux-gnu",
  "managed_dir": "/home/user/.local/share/toolchain-installer/bin",
  "items": [
    {
      "tool": "git",
      "status": "present|installed|failed|unsupported",
      "source": "https://...",
      "destination": "/.../git",
      "detail": "optional detail"
    }
  ]
}
```

## 退出码

- `0`：执行成功；若 `--strict` 未开启，允许部分工具失败。
- `2`：参数错误或不支持的参数组合。
- `3`：下载或校验失败。
- `4`：安装落盘失败。
- `5`：`--strict` 模式下存在失败项。

## 稳定性约束

- `schema_version` 升级前必须保持向后兼容。
- 字段新增只能追加，不得重命名或删除已有字段。
- 调用方应以 `status` + `detail` 处理失败，不依赖 stderr 文本。
- `gateway` 仅在 `country=CN` 且下载目标为 `git release` 时生效。
