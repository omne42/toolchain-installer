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
  - 只有会写入或预留托管根的方法才要求这个目录可解析；纯宿主方法如 `system_package`、`pip`、`workspace_package`、`rustup_component` 不会因为默认托管根不可解析而提前失败。
  - 传给 `npm` / `pnpm` / `bun` / `cargo` / `go` / `uv` 这类宿主命令时，会保留宿主机原生路径字节；Unix 下的非 UTF-8 路径不会在 argv/env 拼装阶段被提前改写。
- `--mirror-prefix <prefix>`
  - 为 `release` / `archive_tree_release` 显式指定下载镜像前缀。
  - 只要 CLI 提供了任意 `--mirror-prefix`，候选顺序就只由这些显式前缀决定；未提供时才会读取 `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES`。
- `--package-index <url>`
  - 为 `uv_tool` 显式指定 Python 包索引。
  - 只要 CLI 提供了任意 `--package-index`，候选顺序就只由这些显式索引决定；未提供时才会读取 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES`。
  - 若 CLI 和环境变量都没有提供任何索引，默认只使用官方 PyPI。
- `--python-mirror <url>`
  - 为 `uv_python` 显式指定备用 Python 下载镜像。
  - 只要 CLI 提供了任意 `--python-mirror`，备用镜像集合就只由这些显式值决定；未提供时才会读取 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS`。
  - 官方来源始终隐式存在。
- `--gateway-base <url>`
  - 外部固定网关入口；installer 本身不包含网关实现。
- `--country <ISO2>`
  - 调用方传入地区码。
- `--max-download-bytes <bytes>`
  - 限制单次 release / bootstrap 下载的最大响应体大小，必须为正整数。
- `--host-recipe-timeout-seconds <seconds>`
  - 覆盖宿主 recipe hard timeout。
  - 作用于 `system_package`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install`，以及 bootstrap 里的系统包 / `pip install uv` fallback；默认是 `900` 秒。
- `--plan-file <path>`
  - 执行 plan 文件。
  - plan JSON 会严格拒绝未知字段；调用方不能依赖拼错字段后被静默忽略。
  - plan 中声明的本地相对路径按该 plan 文件所在目录解析，不按调用 CLI 时的当前工作目录解析；这同样适用于 `pip` / `npm_global` 的本地 `package` 路径。
  - `go_install` 的本地 `package` 既接受 `./cmd/demo`，也接受 `cmd/demo` 这类 bare relative 路径；两者都会按 plan 目录解析，不会被当成远端 module spec。
- `--method <release|archive_tree_release|system_package|apt|pip|npm_global|workspace_package|cargo_install|rustup_component|go_install|uv|uv_python|uv_tool>`
  - 直接参数模式下执行单个安装项。
  - `apt` 是兼容 alias；进入解析层后会直接归位到 `system_package + manager=apt-get`。
- `--id <name>`
  - 单个安装项标识。
  - 对 `npm_global` / `uv_tool` 这类可能出现“包名不等于实际 CLI 名”的方法，若未显式提供 `--binary-name`，installer 也会把 `id` 当作实际 launcher 名的回退提示参与结果解析；`uv_tool` 的幂等 no-op 重跑也会沿用这条提示，而不是只接受“本次有新写 launcher”这一种成功形态。
- `--tool-version <value>`
  - direct-plan 模式里的通用 `version` 字段入口；当前用于 `cargo_install`、`go_install`、`uv_python`。
  - 对 `uv_python` 而言，当前只支持 `3`、`3.13`、`3.13.12` 这类 1 到 3 段的纯数字版本选择器。
  - 对 `cargo_install` 而言，registry package 可以单独传 `version`；若 `package` 解析成本地路径，则必须把版本信息留在该本地源自身，不接受额外 `version`。
  - 对 `go_install` 而言，installer 只会把 `version` 映射到远端 module spec；本地路径或已经自带 `@version` 的 `package` 不接受额外 `version`。
- `--url`、`--sha256`、`--archive-binary`、`--binary-name`、`--destination`
  - `release` 或 `archive_tree_release` 模式字段；其中 `archive_binary` 仅用于 `release`。
  - `--archive-binary` 传的是 archive 内目标二进制的相对路径；installer 会规范斜杠，并在常见单根目录 archive 上自动补齐根目录后再做精确匹配。
  - 非 Windows 宿主会拒绝 `C:\tools\demo.exe` 这类 Windows 绝对 `--destination`，即使 `--target-triple` 是 Windows 也不例外；`destination` 必须符合当前宿主机的实际落盘语义。
  - Windows 宿主会把 `\\server\share\app` 和 `//server/share/app` 都当作 Windows UNC 绝对路径处理；这类路径只适用于允许绝对 `destination` 的方法，不会被误降级成 root-relative。
  - `npm_global`、`cargo_install`、`go_install`、`rustup_component`、`uv_tool` 也允许 `--binary-name`；其中 `rustup_component` 只有在 installer 已知组件对应稳定 CLI 名时才接受这个字段，而且值必须与该 canonical CLI 名一致。安装成功后若该命令不可解析，整项失败，不会静默回退到组件默认二进制。
- `--package`、`--manager`
  - `system_package`、`apt`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install` 或 `uv_tool` 模式字段。
  - 需要固定 canonical `apt-get` 时，可以显式写 `--method apt`；也可以继续写 `--method system_package --manager apt-get`。
  - `pip` / `npm_global` / `workspace_package` / `uv_tool` 的 `--package` 不允许传 `-r`、`--editable`、`--workspace`、`--from` 这类 option-like 值。
- `--python`
  - `pip` 模式的解释器；`uv_tool` 模式的绑定 Python。
  - 对 `pip` 而言，显式提供后只会调用该解释器；未提供时默认先探测 `python3`，只有 `python3` 命令不存在时才回退到 `python`，不会在 `python3 -m pip install` 已经失败后静默切到另一个 Python 环境。
  - `pip` 仍然属于宿主环境变更方法：成功后脚本入口和 site-packages 的实际落点由该解释器自己的环境决定，而不是 `managed_dir`。
- `--json`
  - 输出机器可读 JSON。
- `--strict`
  - 任一安装项失败，或显式请求的 bootstrap 工具返回 `unsupported` 时返回整体非 0。

## 直接参数模式

当调用方只执行一个安装项时，可直接传 `--method` 与对应字段，不必写 JSON plan。

- 只有显式提供 `--method` 时，`--id`、`--tool-version`、`--url`、`--sha256`、`--archive-binary`、`--binary-name`、`--destination`、`--package`、`--manager`、`--python` 这些 direct-plan 字段才合法。
- `--tool-version` 只是 direct-plan 的 CLI 名；进入 plan contract 后对应的仍是通用 `version` 字段，因此只有接受 `version` 的方法才能使用它。
- 若未提供 `--method`，这些字段不会再被静默吞掉后退回 bootstrap；CLI 会直接返回 usage error。
- 若提供了 `--plan-file`，这些 direct-plan 字段同样会被拒绝，而不是继续以“CLI 覆盖 plan”的模糊语义混用。
- `--tool` 只能和纯 bootstrap 模式一起出现；不能再与 `--method` 或 `--plan-file` 混用后被静默忽略。
- `--id` 与 `--binary-name` 都必须是 plain leaf name，不能携带路径分隔符；需要控制目录时应使用允许 `destination` 的方法。

环境变量补充：

- 这些环境变量只在 CLI 边界被读取，并在进入库 API 前显式收敛进 `ExecutionRequest`。
- 纯库调用方如果需要和 CLI 相同的 env fallback 语义，应先自行调用 `ExecutionRequest::with_process_environment_fallbacks()`；否则同一个 request 的行为不会再随进程环境变化，`TOOLCHAIN_INSTALLER_MANAGED_DIR` / `OMNE_DATA_DIR` 也不会再隐式改写 `managed_dir`。

- `TOOLCHAIN_INSTALLER_GITHUB_API_BASES`
  - 逗号分隔的 GitHub metadata API base 列表；未设置时默认只使用 `https://api.github.com`。
- `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES`
  - 逗号分隔的 release 下载镜像前缀；只有当 CLI 没有显式传 `--mirror-prefix` 时才会作为候选顺序输入。
  - 重复值只按首次出现去重。
- `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES`
  - 逗号分隔的 `uv_tool` 显式索引列表；只有当 CLI 没有显式传 `--package-index` 时才会作为候选顺序输入。
  - 若这里和 CLI 都没有提供任何索引，installer 才会回退到官方 PyPI。
  - 重复值只按首次出现去重。
- `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS`
  - 逗号分隔的 `uv_python` 备用镜像列表；只有当 CLI 没有显式传 `--python-mirror` 时才会作为备用镜像输入。
  - 重复值只按首次出现去重。
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
- `TOOLCHAIN_INSTALLER_HOST_RECIPE_TIMEOUT_SECONDS`
  - 覆盖宿主 recipe hard timeout；默认是 `900` 秒。
- `TOOLCHAIN_INSTALLER_HTTP_TIMEOUT_SECONDS`
  - 覆盖 HTTP 总超时；默认是 `120` 秒。
- `TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS`
  - 覆盖 `uv python install` / `uv tool install` 子进程超时；默认是 `900` 秒。
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

当一次 plan 只包含不依赖托管根的方法，且默认 `managed_dir` 又无法从 `--managed-dir`、`TOOLCHAIN_INSTALLER_MANAGED_DIR`、`OMNE_DATA_DIR` 或 `HOME` 推导时，结果里的 `managed_dir` 会返回空字符串；这表示本次执行没有实际使用托管根。

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
  - 单项 direct method / 单项 plan 调用中的下载或校验失败；即使未开启 `--strict` 也会直接返回该 item 的失败码。
- `4`
  - 单项 direct method / 单项 plan 调用中的安装、解压、落盘或宿主安装失败；即使未开启 `--strict` 也会直接返回该 item 的失败码。
- `5`
  - `--strict` 模式下多项执行存在失败项，或显式请求的 bootstrap 工具返回 `unsupported`。

## 稳定性规则

- `schema_version` 升级前必须保持向后兼容。
- 已有输出字段只能追加，不能重命名或删除。
- `status=present` 只表示受支持的内置工具在宿主机上已经发现，且 `--version` 健康检查既成功又匹配该工具预期的版本输出前缀；PATH 中同名但不可执行、探活失败、输出不匹配，或根本不属于支持集的命令名都不会被当成已安装。
- `status=present` 的宿主探测仍只看当前执行上下文里的 PATH，不会因为 `/usr/bin`、`/opt/homebrew/bin` 这类标准安装位置里碰巧已有同名命令就跳过 bootstrap；但如果 `bootstrap --tool git` 已经成功跑完系统包管理器 recipe，installer 会在 PATH 之外补查这些受信任标准位置来确认新装的 `git` 是否真的可用。
- 如果托管目录里已经存在同名内置工具，但该 managed 安装健康检查失败，`bootstrap` 不会因为宿主 PATH 上碰巧也有一个健康同名命令就降级成 `status=present`；它会继续走修复/重装路径，并在失败时保留 `managed_install_broken` 诊断。
- `status=installed` 只在本次 bootstrap 写出的托管 binary 或系统包安装后的宿主命令再次通过同等级健康检查后才成立；下载、解压、launcher 落盘或包管理器命令本身成功但结果仍不可执行时，installer 会返回 `failed`，并在命中托管损坏场景时保留 `managed_install_broken` 诊断，而不是把坏产物报成已安装。
- 调用方应依赖 `status`、`detail` 与 `error_code`，不要解析 stderr 文本。
- stderr 文本是面向人的即时诊断输出，不承诺固定措辞，也不属于机器契约的一部分。
- `source_kind` 是对 `source` 的结构化补充；调用方不应再从 `source` 字符串推断来源类别。
- `cargo_install`、`go_install`、`npm_global`、`workspace_package`、`rustup_component` 这些宿主机 recipe 方法会各自输出同名 `source_kind`，避免调用方再从 `source` 文本推断安装方式。
- `uv_python` 命中官方来源时会输出 `source_kind=canonical`；只有命中显式 Python 镜像时才会输出 `python_mirror`。
- 当 `source_kind=package_index|python_mirror|canonical|mirror|gateway` 且来源 URL 含凭证、query 或 fragment 时，`source` 会输出脱敏后的协议、主机和路径，而不是回显原始敏感 URL。
- `uv_tool` / `uv_python` 失败时若 `detail` 携带有界 stdout/stderr 诊断，其中出现的 `http(s)` URL 也会做同等级脱敏，不回显用户信息、query 或 fragment。
- `archive_match` 仅在安装结果来自 archive 解包时出现；调用方不应再从 `detail` 或日志文本解析匹配到的 archive 内路径。
- `pip` 成功结果里的 `source` 只表示“这次调用用了哪个 Python 解释器”，不表示可重放的 artifact 来源；对应项的 `destination` 会保持为空，调用方不能把它当作 installer 拥有的托管落点。
- `gateway` 仅在 `country=CN` 且下载目标为 `git release` 时生效。
- `gateway` 只会对 canonical `https://github.com/git-for-windows/git/releases/download/...` 下载 URL 生成候选；lookalike URL 不会被字符串匹配误改写。
- `archive_tree_release` 会先把目录树解到 staging 目录，成功后再替换目标目录；失败时不会先删除现有目标内容。
- Windows 上的 `bootstrap --tool git` 会把 MinGit payload 切换和 `git.cmd` launcher 更新作为同一个事务提交；如果 launcher 写入失败，会恢复旧的 `git-portable/` 与旧 launcher，而不是留下半更新状态。

## 继续阅读

- plan schema 与方法矩阵：`install-plan-contract.md`
- 来源选择规则：`../references/source-selection-rules.md`
