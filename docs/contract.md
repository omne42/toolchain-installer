# CLI 契约

## 命令

`toolchain-installer bootstrap [options]`

## 输入参数

- `--tool <name>`：可重复，默认 `git` 和 `gh`。
- `--target-triple <triple>`：可选，覆盖自动探测目标平台。
- `--managed-dir <path>`：可选，安装输出目录。
- `--mirror-prefix <prefix>`：可重复，追加下载候选前缀。
- `--gateway-base <url>`：可选，固定网关入口。
- `--json`：输出机器可读 JSON。
- `--strict`：任一工具失败时返回非 0。

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
