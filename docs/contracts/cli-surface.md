# CLI 表面契约

## 命令形态

- 主命令：`toolchain-installer bootstrap [options]`

对外稳定 CLI 只保留 `bootstrap` 子命令；不再接受裸命令简写。

## 主要参数

- `--tool <name>`
  - 可重复；`bootstrap` 默认安装 `git` 和 `gh`。
  - 只属于 bootstrap 模式；与 `--method`、`--plan-file` 明确互斥，混用会直接返回 usage error。
- `--target-triple <triple>`
  - 覆盖自动探测目标平台。
  - 当前只接受 shared runtime 已知的 canonical 支持集：`x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`x86_64-unknown-linux-musl`、`aarch64-unknown-linux-musl`、`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`。
  - 未知值会在参数校验阶段直接返回 usage error，而不是继续把任意字符串带进执行链路。
- `--managed-dir <path>`
  - 安装输出目录；未指定时默认使用 `~/.omne_data/toolchain/<target>/bin`。
  - 传给 `npm` / `pnpm` / `bun` / `cargo` / `go` / `uv` 这类宿主命令时，会保留宿主机原生路径字节；Unix 下的非 UTF-8 路径不会在 argv/env 拼装阶段被提前改写。
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
  - plan 中声明的本地相对路径按该 plan 文件所在目录解析，不按调用 CLI 时的当前工作目录解析；这同样适用于 `pip` / `npm_global` 的本地 `package` 路径。
- `--method <release|archive_tree_release|system_package|apt|pip|npm_global|workspace_package|cargo_install|rustup_component|go_install|uv|uv_python|uv_tool>`
  - 直接参数模式下执行单个安装项。
- `--id <name>`
  - 单个安装项标识。
- `--tool-version <value>`
  - `uv_python` 直接参数模式下的 Python 版本。
  - 当前只支持 `3`、`3.13`、`3.13.12` 这类 1 到 3 段的纯数字版本选择器。
- `--url`、`--sha256`、`--archive-binary`、`--binary-name`、`--destination`
  - `release` 或 `archive_tree_release` 模式字段；其中 `archive_binary` 仅用于 `release`。
  - `--archive-binary` 传的是 archive 内目标二进制的相对路径；installer 会规范斜杠，并在常见单根目录 archive 上自动补齐根目录后再做精确匹配。
  - 非 Windows 宿主会拒绝 `C:\tools\demo.exe` 这类 Windows 绝对 `--destination`，即使 `--target-triple` 是 Windows 也不例外；`destination` 必须符合当前宿主机的实际落盘语义。
  - `uv_tool` 额外允许 `--binary-name`，用于声明托管目录下期望出现的可执行文件名。
- `--package`、`--manager`
  - `system_package`、`apt`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component` 或 `go_install` 模式字段。
  - `apt` 只接受空 `manager` 或显式 `manager=apt-get`；其他值会在参数校验阶段直接返回 usage error。
  - `pip` / `npm_global` 的 `--package` 不允许传 `-r`、`--editable`、`--workspace` 这类 option-like 值。
- `--python`
  - `pip` 模式的解释器；`uv_tool` 模式的绑定 Python。
  - 对 `pip` 而言，显式提供后只会调用该解释器；未提供时默认先探测 `python3`，只有 `python3` 命令不存在时才回退到 `python`，不会在 `python3 -m pip install` 已经失败后静默切到另一个 Python 环境。
- `--json`
  - 输出机器可读 JSON。
- `--strict`
  - 任一安装项失败，或显式请求的 bootstrap 工具返回 `unsupported` 时返回整体非 0。

## 直接参数模式

当调用方只执行一个安装项时，可直接传 `--method` 与对应字段，不必写 JSON plan。

- 只有显式提供 `--method` 时，`--id`、`--tool-version`、`--url`、`--sha256`、`--archive-binary`、`--binary-name`、`--destination`、`--package`、`--manager`、`--python` 这些 direct-plan 字段才合法。
- 若未提供 `--method`，这些字段不会再被静默吞掉后退回 bootstrap；CLI 会直接返回 usage error。
- 若提供了 `--plan-file`，这些 direct-plan 字段同样会被拒绝，而不是继续以“CLI 覆盖 plan”的模糊语义混用。
- `--tool` 只能和纯 bootstrap 模式一起出现；不能再与 `--method` 或 `--plan-file` 混用后被静默忽略。
- `--id` 与 `--binary-name` 都必须是 plain leaf name，不能携带路径分隔符；需要控制目录时应使用允许 `destination` 的方法。

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
      "source_kind": "gateway|canonical|mirror|managed|system_package|pip|cargo_install|go_install|npm_global|workspace_package|rustup_component|python_mirror|package_index",
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
  - 参数错误、不支持的参数组合，不支持的 target triple，plan 中出现未知字段，或 plan / `--method` 中出现未知方法名。
- `3`
  - 单项调用中的下载或校验失败。
- `4`
  - 单项调用中的安装、解压、落盘或宿主安装失败。
- `5`
  - `--strict` 模式下存在失败项，或显式请求的 bootstrap 工具返回 `unsupported`。

## 稳定性规则

- `schema_version` 升级前必须保持向后兼容。
- 已有输出字段只能追加，不能重命名或删除。
- `status=present` 只表示受支持的内置工具在宿主机上已经发现，且 `--version` 健康检查既成功又匹配该工具预期的版本输出前缀；PATH 中同名但不可执行、探活失败、输出不匹配，或根本不属于支持集的命令名都不会被当成已安装。
- 调用方应依赖 `status`、`detail` 与 `error_code`，不要解析 stderr 文本。
- stderr 文本是面向人的即时诊断输出，不承诺固定措辞，也不属于机器契约的一部分。
- `source_kind` 是对 `source` 的结构化补充；调用方不应再从 `source` 字符串推断来源类别。
- `cargo_install`、`go_install`、`npm_global`、`workspace_package`、`rustup_component` 这些宿主机 recipe 方法会各自输出同名 `source_kind`，避免调用方再从 `source` 文本推断安装方式。
- `uv_python` 命中官方来源时会输出 `source_kind=canonical`；只有命中显式 Python 镜像时才会输出 `python_mirror`。
- 当 `source_kind=package_index|python_mirror|canonical|mirror|gateway` 且来源 URL 含凭证、query 或 fragment 时，`source` 会输出脱敏后的协议、主机和路径，而不是回显原始敏感 URL。
- `archive_match` 仅在安装结果来自 archive 解包时出现；调用方不应再从 `detail` 或日志文本解析匹配到的 archive 内路径。
- `gateway` 仅在 `country=CN` 且下载目标为 `git release` 时生效。
- `archive_tree_release` 会先把目录树解到 staging 目录，成功后再替换目标目录；失败时不会先删除现有目标内容。
- Windows 上的 `bootstrap --tool git` 会把 MinGit payload 切换和 `git.cmd` launcher 更新作为同一个事务提交；如果 launcher 写入失败，会恢复旧的 `git-portable/` 与旧 launcher，而不是留下半更新状态。

## 继续阅读

- plan schema 与方法矩阵：`install-plan-contract.md`
- 来源选择规则：`../references/source-selection-rules.md`
