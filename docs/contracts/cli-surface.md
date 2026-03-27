# CLI 表面契约

## 命令形态

- 主命令：`toolchain-installer bootstrap [options]`

对外稳定 CLI 只保留 `bootstrap` 子命令；不再接受裸命令简写。

## 主要参数

- `--tool <name>`
  - 可重复；`bootstrap` 默认安装 `git` 和 `gh`。
- `--target-triple <triple>`
  - 覆盖自动探测目标平台。
- `--managed-dir <path>`
  - 安装输出目录；未指定时默认使用 `~/.omne_data/toolchain/<target>/bin`。
- `--mirror-prefix <prefix>`
  - 追加 release 下载候选前缀。
- `--package-index <url>`
  - 为 `uv_tool` 显式追加 Python 包索引；若未提供任何索引，默认只使用官方 PyPI。
- `--python-mirror <url>`
  - 追加 `uv_python` 的备用 Python 下载镜像；官方来源隐式存在。
- `--gateway-base <url>`
  - 外部固定网关入口；installer 本身不包含网关实现。
- `--country <ISO2>`
  - 调用方传入地区码。
- `--max-download-bytes <bytes>`
  - 限制单次 release / bootstrap 下载的最大响应体大小，必须为正整数。
- `--plan-file <path>`
  - 执行 plan 文件。
  - plan JSON 会严格拒绝未知字段；调用方不能依赖拼错字段后被静默忽略。
  - plan 中声明的本地相对路径按该 plan 文件所在目录解析，不按调用 CLI 时的当前工作目录解析。
- `--method <release|archive_tree_release|system_package|apt|pip|npm_global|workspace_package|cargo_install|rustup_component|go_install|uv|uv_python|uv_tool>`
  - 直接参数模式下执行单个安装项。
- `--id <name>`
  - 单个安装项标识。
- `--tool-version <value>`
  - `uv_python` 直接参数模式下的 Python 版本。
- `--url`、`--sha256`、`--archive-binary`、`--binary-name`、`--destination`
  - `release` 或 `archive_tree_release` 模式字段；其中 `archive_binary` 仅用于 `release`。
  - `uv_tool` 额外允许 `--binary-name`，用于声明托管目录下期望出现的可执行文件名。
- `--package`、`--manager`
  - `system_package`、`apt`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component` 或 `go_install` 模式字段。
- `--python`
  - `pip` 模式的解释器；`uv_tool` 模式的绑定 Python。
- `--json`
  - 输出机器可读 JSON。
- `--strict`
  - 任一安装项失败时返回整体非 0。

## 直接参数模式

当调用方只执行一个安装项时，可直接传 `--method` 与对应字段，不必写 JSON plan。

环境变量补充：

- `TOOLCHAIN_INSTALLER_GITHUB_API_BASES`
  - 逗号分隔的 GitHub metadata API base 列表；未设置时默认只使用 `https://api.github.com`。
- `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES`
  - 逗号分隔的 release 下载镜像前缀；与 `--mirror-prefix` 共同组成候选顺序。
  - 重复值只按首次出现去重，不会改变显式给定的候选顺序。
- `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES`
  - 逗号分隔的 `uv_tool` 显式索引列表；若这里或 CLI 没有提供任何索引，installer 才会回退到官方 PyPI。
  - 重复值只按首次出现去重，不会改变显式给定的候选顺序。
- `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS`
  - 逗号分隔的 `uv_python` 备用镜像列表。
  - 重复值只按首次出现去重，不会改变显式给定的候选顺序。
- `TOOLCHAIN_INSTALLER_GITHUB_TOKEN`
  - 可选 GitHub token；用于请求 GitHub release metadata API，避免 CI / 共享出口上的匿名限额。若未设置，installer 会回退读取 `GITHUB_TOKEN`。
- `TOOLCHAIN_INSTALLER_GATEWAY_BASE`
  - 当未显式传 `--gateway-base` 时，作为外部固定路由网关基地址。
- `TOOLCHAIN_INSTALLER_COUNTRY`
  - 当未显式传 `--country` 时，作为 gateway 参与条件判断所使用的国家码来源。
- `TOOLCHAIN_INSTALLER_MANAGED_DIR`
  - 直接覆盖默认托管目录。
- `TOOLCHAIN_INSTALLER_MAX_DOWNLOAD_BYTES`
  - 限制单次下载的最大响应体大小；未设置时不启用额外大小上限。
- `TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS`
  - 覆盖 HTTP 总超时；默认是 `120` 秒。
- `OMNE_DATA_DIR`
  - 当未指定 `--managed-dir` 且未设置 `TOOLCHAIN_INSTALLER_MANAGED_DIR` 时，默认托管目录会解析到 `OMNE_DATA_DIR/toolchain/<target>/bin`。

## JSON 输出

```json
{
  "schema_version": 1,
  "host_triple": "x86_64-unknown-linux-gnu",
  "target_triple": "x86_64-unknown-linux-gnu",
  "managed_dir": "/home/user/.omne_data/toolchain/x86_64-unknown-linux-gnu/bin",
  "items": [
    {
      "tool": "git",
      "status": "present|installed|failed|unsupported",
      "source": "https://...",
      "source_kind": "gateway|canonical|mirror|managed|system_package|pip|python_mirror|package_index",
      "archive_match": {
        "format": "tar_gz|tar_xz|zip",
        "path": "archive/entry/path"
      },
      "destination": "/.../git",
      "detail": "optional detail",
      "error_code": "optional machine-readable failure code"
    }
  ]
}
```

`error_code` 当前取值：

- `download_failed`
- `install_failed`
- `managed_install_broken`
- `usage_error`

## 退出码

- `0`
  - 执行成功；若未开启 `--strict`，允许部分工具失败。
- `2`
  - 参数错误、不支持的参数组合，plan 中出现未知字段，或 plan / `--method` 中出现未知方法名。
- `3`
  - 单项调用中的下载或校验失败。
- `4`
  - 单项调用中的安装、解压、落盘或宿主安装失败。
- `5`
  - `--strict` 模式下存在失败项。

## 稳定性规则

- `schema_version` 升级前必须保持向后兼容。
- 已有输出字段只能追加，不能重命名或删除。
- 调用方应依赖 `status`、`detail` 与 `error_code`，不要解析 stderr 文本。
- stderr 文本是面向人的即时诊断输出，不承诺固定措辞，也不属于机器契约的一部分。
- `source_kind` 是对 `source` 的结构化补充；调用方不应再从 `source` 字符串推断来源类别。
- 当 `source_kind=package_index|python_mirror` 且来源 URL 含凭证、query 或 fragment 时，`source` 会输出脱敏后的协议、主机和路径，而不是回显原始敏感 URL。
- `archive_match` 仅在安装结果来自 archive 解包时出现；调用方不应再从 `detail` 或日志文本解析匹配到的 archive 内路径。
- `gateway` 仅在 `country=CN` 且下载目标为 `git release` 时生效。
- `archive_tree_release` 会先把目录树解到 staging 目录，成功后再替换目标目录；失败时不会先删除现有目标内容。

## 继续阅读

- plan schema 与方法矩阵：`install-plan-contract.md`
- 来源选择规则：`../references/source-selection-rules.md`
