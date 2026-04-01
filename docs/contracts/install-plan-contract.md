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
- `plan.items[*].id` 还是默认目标目录/文件名的兜底输入，因此必须是 plain leaf name，不能包含 `/`、`\`、`.`、`..` 或 `C:foo` 这类 path-like 片段；调用方不应再把 `id` 当作隐式子目录。
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
  - 若调用方显式提供 `python`，installer 只会使用该解释器，不会再静默回退到别名命令；只有未提供 `python` 时，才会按默认宿主候选顺序尝试。
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
  - 允许 `version`，但当前只支持 `3`、`3.13`、`3.13.12` 这类 1 到 3 段的纯数字版本选择器。
- `uv_tool`
  - 允许 `package`、可选 `python`、可选 `binary_name`。
  - 所有 `binary_name` 都必须是 plain file name；调用方如果想控制子目录，应使用对应方法允许的 `destination`，而不是把路径片段塞进 `binary_name`。

## 宿主与目标约束

- `target_triple` 只接受 shared runtime 已知的 canonical target triple；未知值会在结构校验前直接返回 usage error。
- 当前支持的 canonical target triple 只有：`x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`x86_64-unknown-linux-musl`、`aarch64-unknown-linux-musl`、`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`。
- `bootstrap` 仅支持当前宿主机，即 `target_triple` 必须等于自动探测到的 `host_triple`。
- `method=release` 支持显式跨目标平台下载与落盘。
- `method=archive_tree_release` 支持显式跨目标平台下载与解包。
- `method=system_package|apt|pip|npm_global|workspace_package|cargo_install|rustup_component|go_install|uv|uv_python|uv_tool` 仅作用于当前宿主机。
- 若宿主机方法出现 `target_triple != host_triple`，执行前直接返回退出码 `2`。

## 路径与 URL 约束

- `release.url` 仅允许 `http` 或 `https`。
- `archive_tree_release.url` 仅允许 `http` 或 `https`，且资产名必须是受支持的 `.tar.gz`、`.tar.xz` 或 `.zip`；不支持的资产名会在结构校验阶段直接以 usage error 拒绝。
- 对 `release`、`archive_tree_release` 等托管落盘方法，`destination` 若为相对路径，会解析到 `managed_dir` 下。
- 托管落盘方法的 Unix 风格绝对路径如 `/tmp/demo` 会被拒绝，避免绕过托管目录边界。
- 任意 `destination` 都禁止包含 `..`，避免路径逃逸。
- 为了避免 Windows 语义下的伪相对路径逃逸，`destination` 还禁止使用 `C:foo` 这类 drive-relative 路径，以及 `\foo` 这类 root-relative 路径。
- 托管落盘方法同样拒绝 `C:\tools\demo.exe` 这类 Windows 绝对路径；不能借“宿主机本身是 Windows”或“显式把 `target_triple` 设成 Windows”把写入绕出 `managed_dir`。
- `workspace_package` 同样只在 Windows 宿主上接受 `C:\workspace\app` 这类 Windows 绝对目录；非 Windows 宿主必须使用当前宿主机有效的绝对路径语法。
- `release` 未指定 `destination` 时，默认安装到 `managed_dir/<binary_name>`。
- `release.archive_binary` 表示 archive 内目标二进制的相对路径；installer 会先规范斜杠，并在常见“单一根目录” archive 上自动补齐该根目录，兼容 shared runtime 当前要求的精确 archive 路径匹配。
- `archive_tree_release` 未指定 `destination` 时，默认解到 `managed_dir/<id>/`。
- `archive_tree_release` 会先把 archive 解到同级 staging 目录，只有校验和解包都成功后才替换目标目录；失败时不会先删除现有内容。
- `workspace_package` 必须显式给出 `destination`，并把它当作工作区目录路径；绝对路径会原样使用，相对路径则按 plan 文件所在目录解析，不会默认写入 `managed_dir`。
- `workspace_package` 不接受独立 `version` 字段；如需锁定版本，应直接把版本写进 `package` 自身。
- `system_package`、`apt` 的 `package` 会先按 shared runtime 的 `SystemPackageName` 校验；空串、任何空白、控制字符、路径分隔符、`.`/`..` 以及看起来像 option 的值会在执行前直接返回 install error，而不是继续拼进包管理器 argv。
- `pip`、`npm_global`、`workspace_package`、`cargo_install`、`go_install`、`rustup_component`、`uv_tool` 的 `package` 不允许是 `-r`、`--editable`、`--workspace`、`--git`、`--toolchain`、`--index-url` 这类看起来像命令行选项的值；installer 会在 resolve 阶段直接返回 usage error，而不是把额外语义透传给底层包管理器。
- 多个 `workspace_package` item 可以指向同一个 workspace；这表示对同一工作区重复执行依赖安装，不会因为“目标目录相同”在执行前被当成互斥输出拦下。
- `npm_global`、`cargo_install`、`go_install` 的最终可执行文件路径以结果里的 `destination` 为准；调用方不应假设它们都严格等于 `managed_dir/<binary>`。
- `npm_global` 使用 `bun` 时，若 `managed_dir` 本身已经是 `.../bin`，installer 会直接把它当作 bun 的全局 binary 目录，而不是再额外套一层 `bin/` 形成 `.../bin/bin/<tool>`。
- `npm_global`、`cargo_install`、`go_install`、`uv_tool` 若未显式提供 `binary_name`，installer 会优先从 `package` 或解析后的本地/远端来源推导默认二进制名；只有确实推不出来时才回退到 `id`。
- `npm_global` 若 `package` 使用 `npm:` alias source spec，默认 `binary_name` 会从 alias 指向的真实包名推导，并剥离 `@1.2.3` 这类内嵌版本片段；`github:`、`git:`、`file:` 等 source spec 则只取仓库或路径叶子名，不会把整段 source spec 当成文件名。
- `cargo_install`、`go_install` 若显式提供 `binary_name`，它表示最终托管目标文件名；installer 会先在隔离 staging 目录执行安装，再把本次实际产出的目标二进制提升到该文件名。只要 staging 产物里没有名字直接匹配请求的 `binary_name`，整项就会返回失败而不是猜测，即使 staging 里只剩一个名字不匹配的二进制也一样。
- `cargo_install` 若显式提供 `binary_name`，installer 还会把它传给 `cargo install --bin <binary_name>`；如果上游 crate 根本不导出这个可执行文件，整项会直接失败，而不是靠旧文件误报成功。
- Windows target 下，`npm_global` 的 `pnpm`/`bun` CLI 入口也按 `<binary>.cmd` 参与目标路径冲突校验，而不是按无后缀文件名比较。
- `npm_global` 的幂等重跑允许包管理器 no-op，但前提是 installer 还能从托管目录里的 manifest/bin 元数据按 package spec 语义证明“当前安装与请求相符”：精确版本仍要求 `manifest.version` 精确匹配，`latest`/range/tag/`npm:` alias 以及 `github:`/`git:`/`file:` 等显式来源则退回到包名或可解析来源身份匹配；孤儿旧 binary 仍不会被当成成功安装。
- `npm_global` 做幂等探测时会按目标平台使用原生包目录布局：npm 在 Unix 上检查 `<prefix>/lib/node_modules`，在 Windows target 上检查 `<prefix>/node_modules`；pnpm 会递归扫描 `PNPM_HOME/global/` 下的实际 store/workspace 布局，而不是假设固定单层目录。
- `npm_global` 在托管目录内做包目录或回退二进制探测时不会跟随目录 symlink，避免循环目录把安装流程拖死；同时会跳过不可读目录、不可读 manifest 和坏 manifest，不会因为单个坏条目把幂等重跑误判成失败。
- `npm_global` 为了兼容重复安装留下的 managed leaf symlink，会允许目标 leaf 自身是 symlink；但 symlink 解析后的最终路径仍必须落在 `managed_dir` 内，指向托管根外部的 leaf symlink 会在执行前直接拒绝。
- `cargo_install` 若 `managed_dir` 本身不是 `bin/` 目录，结果二进制会落到 `managed_dir/bin/<binary>`，不会越过调用方给定的托管根。
- plan 文件中的本地相对路径输入（例如 `cargo_install` 的本地包路径、`go_install` 的 `./cmd/demo` 或 `cmd/demo` 这类裸相对源码路径、`workspace_package` 的相对工作区目录，以及 `pip`/`npm_global` 的本地 `package` 路径）按 plan 文件所在目录解析；解析时会先对 plan 基准目录做词法规范化，不会因为 `plans/../plans` 这类等价写法绕过后续目标冲突校验。
- 这类“按本地路径解释”的 `package` 输入也只接受当前宿主机原生的绝对路径语法：非 Windows 宿主会在 resolve 阶段直接拒绝 `C:\repo\demo`、`\repo\demo`、`file:C:\repo\demo` 这类 Windows-local 路径，而不是把它们误当成相对路径继续执行。
- `workspace_package` 的工作区目录、`cargo_install` / `go_install` / `npm_global` 的托管 staging 或 prefix 路径，以及 `uv_python` / `uv_tool` 注入的 `UV_*` 目录环境变量，都会按宿主机原生路径字节传给子进程；installer 不会先做 UTF-8 round-trip 再拼 argv 或 env。
- 当目标是 Windows 时，相对 `destination` 会按 Windows 路径分隔语义归一化；`bin\\tool.exe` 和 `bin/tool.exe` 会落到同一个托管相对路径，不再依赖当前宿主机是否把反斜杠当普通字符。
- `uv_tool` 若提供 `binary_name`，结果里的 `destination` 会指向该二进制在 `managed_dir` 下的实际路径；安装成功后若该路径不存在，整项返回失败。
- `uv_tool` 若显式提供 `binary_name`，installer 会改用 `uv tool install --from <package> <binary_name>`，把请求的可执行文件名直接纳入上游安装契约，而不是只在安装后被动检查结果路径。
- `cargo_install`、`go_install`、`uv_tool` 在替换同名目标路径时，会先按最终 `destination` 获取同级 advisory lock，再把旧路径整体暂存到 canonical backup；并发 installer 命中同一目标时会串行等待，不会因为共享固定备份名而互相删掉对方的新产物。旧路径无论原先是文件还是目录，安装成功后都会清理备份，失败时恢复原状，不会把目录误当文件导致残留 `.toolchain-installer-backup`。
- 如果上一次安装在 stash 之后异常中断，下一次 `cargo_install`、`go_install`、`uv_tool` 重试会先用 canonical backup 自愈恢复旧目标，再重新暂存；如果成功路径上的旧 backup 已经只剩清理残留，则会自动移到同级 `*.stale-*` 隔离名，避免后续重试继续被固定备份名卡死。
- `go_install` 若 `package` 解析成本地目录，会先校验该目录真实存在且是目录，再去探测 `go` 命令、创建 staging root 或暂存现有目标；无效输入不会先破坏已有托管 binary。
- 所有可确定最终输出路径的方法都参与全局冲突校验；两个 item 不能指向同一路径，也不能形成父子路径重叠，避免后执行项覆盖先执行项目录树。
- 对 Windows target，冲突校验始终按大小写不敏感语义比较路径；对 Darwin target，则按目标路径所在宿主文件系统的真实大小写语义比较，不再把所有 macOS 卷一律当成大小写不敏感。
- `uv_python` 会占用 `managed_dir/.uv-python` 这块托管安装根，并预留它在 `managed_dir` 顶层实际可能写出的 `python` / `python3` / `python3.x` shim 名称；因此它会继续拦截其他方法写入这棵子树或这些顶层解释器入口，但多个 `uv_python` item 彼此不会仅因共享这些托管路径就在执行前互相冲突。
- `uv`、`uv_python`、`uv_tool` 只有在已有托管 `uv` 通过 `--version` 健康检查后才会直接复用；若托管 `uv` 文件存在但健康检查失败，会先自愈重装再继续执行。
- `uv` 方法始终保证结果落到 `managed_dir/uv[.exe]`；即使宿主机已装了健康的 `uv`，它也不会把宿主二进制直接当成 `uv` item 的安装结果。
- `uv_python`、`uv_tool` 在缺少健康托管 `uv` 时，会先按顺序尝试复用健康 host `uv`、再尝试用宿主 `python -m pip install --target ... uv` 在 `managed_dir/.uv-bootstrap/` 下自举一个可复用 `uv`；只有这些本地可复用路径都失败后，才会回退到 GitHub public release 下载独立 `uv` 二进制。
- `uv_python` 只有在 `managed_dir` 下实际发现匹配版本的 Python 可执行文件后才算成功；单纯 `uv python install` 退出码为 `0` 不构成成功条件。
- `uv_python` 的版本匹配按版本段比较：请求 `3` 可以接受托管目录里的任意 `3.x.y`，请求 `3.13` 可以接受 `3.13.x`，但请求 `3.13.1` 不会误接受 `3.13.12`。
- `uv_python` 当请求 `3` 或 `3.13` 这类 family selector 时，会在所有匹配的托管解释器里选择版本最高的那个，不会因为目录字典序或旧安装残留而回退到更老的 patch 版本。
- `uv_tool` 若目标路径上已有同名旧二进制，installer 会先把旧文件挪到临时备份；只有本次 `uv tool install` 真正产出新的目标二进制，且该入口还能通过一次带超时上限的 `--version` 健康探测后才算成功，失败时会恢复旧文件。

## 来源探测与回退

- `uv_tool`
  - 当调用方没有显式提供任何包索引时，默认只使用官方 PyPI `https://pypi.org/simple`。
  - 当调用方显式提供了 `--package-index` 或 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES` 时，installer 不再隐式把官方 PyPI 插到最前面；显式索引顺序就是候选顺序。
  - 若显式索引、镜像或镜像前缀里出现重复值，只会保留第一次出现的位置，不会按字典序重排。
  - 安装前会探测显式索引的可达性，把可达源优先用于安装。
  - 结果里的 `source` 会对显式索引做脱敏，只保留协议、主机和路径，不回显 URL 中的用户信息、query 或 fragment。
  - 调用时会显式移除宿主进程继承的 `UV_*` 环境变量，只保留 installer 自己注入的托管目录布局和显式索引配置，避免外部 shell 状态静默污染来源选择。
- `uv_python`
  - 会先尝试官方 Python 下载来源，失败后再按顺序回退到 `--python-mirror` 或 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS` 提供的备用站。
  - 备用镜像列表若有重复值，只保留第一次出现的位置。
  - 官方来源成功时，结果里的 `source_kind` 会是 `canonical`；只有显式镜像命中时才会是 `python_mirror`。
  - 结果里的 `source` 会对显式镜像做脱敏，只保留协议、主机和路径，不回显 URL 中的用户信息、query 或 fragment。
  - 调用时会显式移除宿主进程继承的 `UV_*` 环境变量，只保留 installer 自己注入的托管目录布局和显式 Python mirror 配置，避免外部 shell 状态静默污染来源选择。
- `release`、`archive_tree_release`
  - 资产类型判断基于 URL 的 path 最后一段，不把 query string 当成资产名的一部分；`tool.tar.gz?download=1` 仍按 `tool.tar.gz` 处理。
  - 结果里的 `source` 会对最终命中的下载 URL 做脱敏，只保留协议、主机和路径，不回显用户信息、query 或 fragment。
- `release`
  - 通过内置来源规则、镜像前缀与可达性结果确定下载候选顺序。
  - `--mirror-prefix` 与 `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES` 的重复值只去重，不重排显式顺序。

## 参考输入

- Python 工具链组合 plan：`../../examples/python-plan.json`
- 单独安装 `uv`：`../../examples/uv-plan.json`
- 单独安装 `ruff`：`../../examples/ruff-plan.json`
- 单独安装 `mypy`：`../../examples/mypy-plan.json`

这些文件是可执行参考，不应在文档里维护第二套不同版本的示例。
