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

- `schema_version` 是必填字段，当前固定为 `1`。
- `plan.items` 不能为空。
- 顶层对象和每个 `items[]` 对象都启用严格未知字段校验；拼错字段名会在反序列化阶段直接失败，不会被静默吞掉。
- `plan.items[*].id` 必须全局唯一；重复 `id` 会在执行前返回退出码 `2`。
- `plan.items[*].id` 还是默认目标目录/文件名的兜底输入，因此必须是 plain leaf name，不能包含 `/`、`\`、`.`、`..` 或 `C:foo` 这类 path-like 片段；调用方不应再把 `id` 当作隐式子目录。
- `method` 必须是受支持的方法名；未知方法会在执行前直接返回退出码 `2`。
- 不属于该方法的字段组合会在执行前返回退出码 `2`，不会静默忽略。
- 纯库 API `validate_install_plan()` 只做 schema、字段组合、宿主/目标约束和重复 `id` 校验；它不知道 `managed_dir`，因此不会擅自猜测依赖目标目录的全局路径冲突。
- CLI 与 `validate_install_plan_with_request()` 会在结构校验通过后，再结合真实 `managed_dir` 做需要托管根的方法的全局目标路径冲突校验；如果整份 plan 只包含 `system_package`、`pip`、`workspace_package`、`rustup_component` 这类不依赖托管根的方法，即使默认 `managed_dir` 不可解析也不会提前失败。
- 纯库调用若要传 `ExecutionRequest.plan_base_dir`，该路径必须已经是绝对路径；库层不会再偷偷回退到进程当前工作目录帮调用方猜测基准目录。
- 纯库调用若希望复用 CLI 对 `TOOLCHAIN_INSTALLER_MANAGED_DIR`、`OMNE_DATA_DIR` 等环境变量的 fallback 语义，需要先显式调用 `ExecutionRequest::with_process_environment_fallbacks()`；执行层不会再直接从这两个环境变量隐式改写 request 的 `managed_dir`。
- 真正进入 bootstrap 或 plan 执行时，installer 还会对目标 `managed_dir` 获取进程级 advisory lock；命中同一托管根的并发调用会串行等待，直到前一个执行释放锁后才继续，避免共享 state root、固定 staging 名或回滚逻辑互相踩坏。
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
- `system_package`
  - 归属于宿主系统包安装域。
- `apt`
  - 归属于宿主系统包安装域。
  - 它是固定 `apt-get` 的显式 alias；执行层会收敛成 `system_package + manager=apt-get`，不会再让调用方自己拼 manager 字符串猜测行为。
- `pip`
  - 归属于 Python 包安装域。
  - 它表达的是“把包交给选定解释器所在环境执行 `python -m pip install`”这一宿主环境变更，不承诺把产物收口到 installer 自己可拥有的托管目标路径。
  - 若调用方显式提供 `python`，installer 只会使用该解释器，不会再静默回退到别名命令。
  - 若未提供 `python`，installer 默认先尝试 `python3`；只有 `python3` 命令不存在时，才会回退到 `python`，不会在前一个解释器已经执行失败后继续静默切换环境。
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
- direct CLI 模式里的 `--tool-version` 只是这个通用 `version` 字段的参数名；它不只属于 `uv_python`，同样可映射到 `cargo_install`、`go_install`。

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
- 为了避免 Windows 语义下的伪相对路径逃逸，`destination` 还禁止使用 `C:foo` 这类 drive-relative 路径，以及 `\foo`、`/foo` 这类 root-relative 路径。
- 托管落盘方法同样拒绝 `C:\tools\demo.exe` 这类 Windows 绝对路径；不能借“宿主机本身是 Windows”或“显式把 `target_triple` 设成 Windows”把写入绕出 `managed_dir`。
- 当前 host triple 若使用 Windows 路径语义，则 `\\server\share\app` 与 `//server/share/app` 这两种 UNC 绝对路径写法都会按 Windows absolute path 处理：`workspace_package` 可以显式使用它们，托管落盘方法则继续统一拒绝。
- `release` 未指定 `destination` 时，默认安装到 `managed_dir/<binary_name>`。
- `release.archive_binary` 表示 archive 内目标二进制的相对路径；installer 会先规范斜杠，并在常见“单一根目录” archive 上自动补齐该根目录，兼容 shared runtime 当前要求的精确 archive 路径匹配。
- `archive_tree_release` 未指定 `destination` 时，默认解到 `managed_dir/<id>/`。
- `archive_tree_release` 会先把 archive 解到同级 staging 目录，只有校验和解包都成功后才替换目标目录；失败时不会先删除现有内容。
- `workspace_package` 必须显式给出 `destination`，并把它当作工作区目录路径；绝对路径会原样使用，相对路径则按 plan 文件所在目录解析，不会默认写入 `managed_dir`。
- `workspace_package` 若使用 Windows 绝对 `destination`，只有当当前 host triple 使用 Windows 路径语义时才会接受；是否下载 Windows target 资产不改变宿主机文件系统的绝对/相对路径语义。
- `workspace_package` 执行时会把底层包管理器的工作目录锚定到该 workspace；即使调用 CLI 时的当前目录不同，`npm` 的 `file:`、相对路径和其他依赖解析也按目标 workspace 解析，而不是按 installer 进程当前目录漂移。
- `workspace_package` 不接受独立 `version` 字段；如需锁定版本，应直接把版本写进 `package` 自身。
- `system_package`、`apt` 的 `package` 会先按 shared runtime 的 `SystemPackageName` 校验；空串、任何空白、控制字符、路径分隔符、`.`/`..` 以及看起来像 option 的值会在执行前直接返回 install error，而不是继续拼进包管理器 argv。
- `method=system_package` 若显式传 `manager=apt-get`，会固定收敛到 canonical `apt-get` recipe；`method=apt` 则直接固定到同一 canonical `apt-get` recipe，并只接受可选 `manager=apt-get`。
- `pip`、`npm_global`、`workspace_package`、`cargo_install`、`go_install`、`rustup_component`、`uv_tool` 的 `package` 不允许是 `-r`、`--editable`、`--workspace`、`--git`、`--toolchain`、`--index-url` 这类看起来像命令行选项的值；installer 会在 resolve 阶段直接返回 usage error，而不是把额外语义透传给底层包管理器。
- `pip` 成功结果里的 `source` 只会记录实际使用的解释器标识（例如 `pip:python3` 或 `pip:/abs/path/python3.13`），不会把它包装成 artifact 坐标；同时 `destination` 会保持为空，因为底层 site-packages / script 落点由被选中的 Python 环境决定，而不是 installer 自己拥有的托管输出。
- 多个 `workspace_package` item 只有在 `manager` 相同的前提下才可以指向同一个 workspace；这表示对同一工作区重复执行同一套包管理器的依赖安装，不会因为“目标目录相同”在执行前被当成互斥输出拦下。
- 如果同一个 workspace 在一份 plan 里混用 `npm`、`pnpm`、`bun`，installer 会在执行前直接返回退出码 `2`，避免把不同 lockfile / store 语义叠到同一工作区里。
- `npm_global`、`cargo_install`、`go_install` 的最终可执行文件路径以结果里的 `destination` 为准；调用方不应假设它们都严格等于 `managed_dir/<binary>`。
- `npm_global` 使用 `bun` 时，若 `managed_dir` 本身已经是 `.../bin`，installer 会直接把它当作 bun 的全局 binary 目录，而不是再额外套一层 `bin/` 形成 `.../bin/bin/<tool>`。
- `npm_global`、`cargo_install`、`go_install`、`uv_tool` 若未显式提供 `binary_name`，installer 会优先从 `package` 或解析后的本地/远端来源推导默认二进制名；只有确实推不出来时才回退到 `id`。
- `npm_global`、`cargo_install`、`go_install` 若 `package` 已经编码了版本、显式来源或本地路径，就不接受额外 `version` 字段；installer 会在 resolve 阶段直接返回 usage error，而不是静默忽略其中一边。
- `go_install` 把 `./cmd/demo` 这类显式相对路径和 `cmd/demo` 这类 bare relative 工作区路径都当作本地包路径处理；它们会按 plan 文件所在目录解析，而不是被当成远端 module spec 走网络安装。
- Windows target 下，如果 `npm_global`、`cargo_install`、`go_install`、`uv_tool` 显式提供的 `binary_name` 已经带有 `.cmd` 或 `.exe` 这类平台后缀，执行层不会再重复追加同一后缀。
- `npm_global` 若未显式提供 `binary_name`，且包名推导出的默认入口在托管 bin 目录里并不存在，installer 会继续结合 item `id` 与已安装包的 manifest/bin 元数据解析真实 CLI 入口；像 `typescript -> tsc` 这类“包名不等于命令名”的安装不会再被误判成失败。
- `npm_global` 若 `package` 使用 `npm:` alias source spec，默认 `binary_name` 会从 alias 指向的真实包名推导，并剥离 `@1.2.3` 这类内嵌版本片段；`github:`、`git:`、`file:` 等 source spec 则只取仓库或路径叶子名，不会把整段 source spec 当成文件名。
- `cargo_install`、`go_install` 若显式提供 `binary_name`，它表示最终托管目标文件名；installer 会先在隔离 staging 目录执行安装，再把本次实际产出的目标二进制提升到该文件名。只要 staging 产物里没有名字直接匹配请求的 `binary_name`，整项就会返回失败而不是猜测，即使 staging 里只剩一个名字不匹配的二进制也一样。
- `cargo_install` 若显式提供 `binary_name`，installer 还会把它传给 `cargo install --bin <binary_name>`；如果上游 crate 根本不导出这个可执行文件，整项会直接失败，而不是靠旧文件误报成功。
- `rustup_component` 若显式提供 `binary_name`，installer 只会在已知组件对应稳定 CLI 名时接受，而且该值必须与 canonical CLI 名一致；例如 `rustfmt -> rustfmt`、`clippy -> cargo-clippy`。对 `rust-src` 这类不产出稳定 CLI 的组件，installer 会在 resolve 阶段直接拒绝 `binary_name`，避免把不相关 PATH 命令误报成组件产物。
- `rustup_component` 一旦接受了 `binary_name`，就会把它当作结果契约里的权威二进制名；安装成功后若该命令仍不可解析，整项会返回失败，而不是静默回退到组件默认二进制。
- Windows target 下，`npm_global` 的 `pnpm`/`bun` CLI 入口也按 `<binary>.cmd` 参与目标路径冲突校验，而不是按无后缀文件名比较。
- `npm_global` 的预检冲突校验不只看最终 CLI binary，还会保留包管理器自己的托管状态树：`npm` 保留 prefix 下的 `node_modules` 根，`bun` 保留 `install/global/node_modules`，`pnpm` 保留其托管 `global` 根。多个 `npm_global` item 可以共享同一类内部状态根，但其他方法不能把目标落到这些树里。
- `npm_global` 的幂等重跑允许包管理器 no-op，但前提是 installer 还能从托管目录里的 manifest/bin 元数据，或上一次成功安装时写下的 installer-managed receipt，按 package spec 语义证明“当前安装与请求相符”，并重新解析出与结果 `destination` 相同的 CLI 入口：精确版本仍要求 `manifest.version` 精确匹配，`latest`/range/tag/`npm:` alias 以及 `github:`/`git:`/`file:` 等显式来源则退回到包名或可解析来源身份匹配；对 `file:`、`github:`、`git:`、本地路径这类显式来源，若只能从 spec 里稳定解析出仓库名或路径叶子名，installer 也只会接受 manifest `name` 与这个可解析身份一致的包目录，不会把 search root 下无关包的 metadata 当成命中。孤儿旧 binary，或名字刚好还在但 manifest 当前已指向别的 bin 的旧 wrapper，都不会被当成成功安装。
- `npm_global` 做幂等探测时会按目标平台使用原生包目录布局：npm 在 Unix 上检查 `<prefix>/lib/node_modules`，在 Windows target 上检查 `<prefix>/node_modules`；pnpm 会递归扫描 `PNPM_HOME/global/` 下的实际 store/workspace 布局，而不是假设固定单层目录。
- `npm_global` 在托管目录内做包目录或回退二进制探测时不会跟随目录 symlink，避免循环目录把安装流程拖死；同时会跳过不可读目录、不可读 manifest 和坏 manifest，不会因为单个坏条目把幂等重跑误判成失败。
- `npm_global` 为了兼容重复安装留下的 managed leaf symlink，会允许目标 leaf 自身是 symlink；但 symlink 解析后的最终路径仍必须落在 `managed_dir` 内。无论这个 symlink 是安装前就已存在，还是本次安装新写出的，只要最终解析到托管根外部，整项都会失败。
- `cargo_install` 若 `managed_dir` 本身不是 `bin/` 目录，结果二进制会落到 `managed_dir/bin/<binary>`，不会越过调用方给定的托管根。
- plan 文件中的本地相对路径输入（例如 `cargo_install` 的本地包路径、`go_install` 的 `./cmd/demo` 或 `cmd/demo` 这类裸相对源码路径、`workspace_package` 的相对工作区目录，以及 `pip`/`npm_global` 的本地 `package` 路径）按 plan 文件所在目录解析；解析时会先对 plan 基准目录做词法规范化，不会因为 `plans/../plans` 这类等价写法绕过后续目标冲突校验。
- 这类“按本地路径解释”的 `package` 输入也只接受当前宿主机原生的绝对路径语法：非 Windows 宿主会在 resolve 阶段直接拒绝 `C:\repo\demo`、`\repo\demo`、`file:C:\repo\demo` 这类 Windows-local 路径，而不是把它们误当成相对路径继续执行。
- `workspace_package` 的工作区目录、`cargo_install` / `go_install` / `npm_global` 的托管 staging 或 prefix 路径，以及 `uv_python` / `uv_tool` 注入的 `UV_*` 目录环境变量，都会按宿主机原生路径字节传给子进程；installer 不会先做 UTF-8 round-trip 再拼 argv 或 env。
- 当当前 host triple 是 Windows 时，相对 `destination` 会按 Windows 路径分隔语义归一化；仅仅把 `target_triple` 设成 Windows，不会让 Unix 宿主机把反斜杠改解释成目录分隔符。
- `uv_tool` 若提供 `binary_name`，结果里的 `destination` 会指向该二进制在 `managed_dir` 下的实际路径；安装成功后若该路径不存在，整项返回失败。
- `uv_tool` 若显式提供 `binary_name`，installer 会改用 `uv tool install --from <package> <binary_name>`，把请求的可执行文件名直接纳入上游安装契约，而不是只在安装后被动检查结果路径。
- `uv_tool` 若提供 `python=3`、`3.13`、`3.13.12` 这类纯版本选择器，installer 会优先把它解析到已存在的托管 Python 可执行文件；只有当前托管根里还没有匹配解释器时，才会把它映射成带平台/`libc` 约束的完整 Python request 传给 `uv`，避免上游在 Linux 上自行选错 `gnu` / `musl` 变体。
- `uv_tool` 若未显式提供 `binary_name`，且按包名推导出的默认入口不存在，installer 会优先检查 item `id` 对应的托管入口，再在本次新建/更新的托管 launcher 里按稳定顺序选择实际 CLI；像 `httpie -> http` 这类 distribution 名与命令名不同的包不会再被误判成失败。
- `uv_tool` 的幂等 no-op 重跑同样会沿用这条 `id` 回退提示语义：如果本次安装没有改写 launcher，但托管目录里已经存在一个与 item `id` 匹配且健康的既有入口，installer 会把它视为当前请求对应的 CLI，而不是把无变化的重跑误报为失败。
- `cargo_install`、`go_install`、`uv_tool` 在替换同名目标路径时，会先按最终 `destination` 获取同级 advisory lock，再把旧路径整体暂存到 canonical backup；并发 installer 命中同一目标时会串行等待，不会因为共享固定备份名而互相删掉对方的新产物。旧路径无论原先是文件还是目录，安装成功后都会清理备份，失败时恢复原状，不会把目录误当文件导致残留 `.toolchain-installer-backup`。
- 如果上一次安装在 stash 之后异常中断，下一次 `cargo_install`、`go_install`、`uv_tool` 重试会先用 canonical backup 自愈恢复旧目标，再重新暂存；如果成功路径上的旧 backup 只剩清理残留，哪怕残留的是 dangling symlink，这些条目也会自动移到同级 `*.stale-*` 隔离名，避免后续重试继续被固定备份名卡死。
- `go_install` 若 `package` 解析成本地目录，会先校验该目录真实存在且是目录，再去探测 `go` 命令、创建 staging root 或暂存现有目标；无效输入不会先破坏已有托管 binary。
- 所有可确定最终输出路径的方法都参与全局冲突校验；两个 item 不能指向同一路径，也不能形成父子路径重叠，避免后执行项覆盖先执行项目录树。
- 冲突校验的大小写语义跟随目标路径实际落盘的宿主文件系统，而不是只看 `target_triple`：Windows host 继续按大小写不敏感语义比较；Darwin host 则按目标路径所在卷的真实大小写语义比较；把 `target_triple` 设成 Windows 并不会让非 Windows 宿主提前按 Windows 文件系统规则拒绝本来合法的大小写不同路径。
- `uv_python` 会占用 `managed_dir/.uv-python` 这块托管安装根，并预留它在 `managed_dir` 顶层实际可能写出的 `python` / `python3` / `python3.x` shim 名称；因此它会继续拦截其他方法写入这棵子树或这些顶层解释器入口，但多个 `uv_python` item 彼此不会仅因共享这些托管路径就在执行前互相冲突。
- `uv_python`、`uv_tool` 的预检冲突校验还会保留共享托管状态根：`.uv-bootstrap`、`.uv-cache`、`.uv-python`，以及 `uv_tool` 额外使用的 `.uv-tools`。托管工具链方法之间允许共享这些根，但其他方法不能把目标写到这些树下。
- `uv`、`uv_python`、`uv_tool` 只有在已有托管 `uv` 通过 `--version` 健康检查后才会直接复用；若托管 `uv` 文件存在但健康检查失败，会先自愈重装再继续执行。
- `uv` 方法始终保证结果落到 `managed_dir/uv[.exe]`；即使宿主机已装了健康的 `uv`，它也不会把宿主二进制直接当成 `uv` item 的安装结果。
- `uv_python`、`uv_tool` 在缺少健康托管 `uv` 时，会先按顺序尝试复用健康 host `uv`、再尝试用宿主 `python -m pip install --target ... uv` 在 `managed_dir/.uv-bootstrap/` 下自举一个可复用 `uv`；只有这些本地可复用路径都失败后，才会回退到 GitHub public release 下载独立 `uv` 二进制。
- `uv_python` 只有在 `managed_dir` 下实际发现“本次新建或更新”的匹配版本 Python 可执行文件后才算成功；单纯 `uv python install` 退出码为 `0`，或目录里本来就残留着匹配版本旧解释器，都不构成成功条件。
- `uv_python` 的版本匹配按版本段比较：请求 `3` 可以接受托管目录里的任意 `3.x.y`，请求 `3.13` 可以接受 `3.13.x`，但请求 `3.13.1` 不会误接受 `3.13.12`。
- `uv_python` 调用 `uv python install` 时会把 canonical `target_triple` 显式映射成完整 Python request（例如 `cpython-3.13.12-linux-x86_64-gnu`），避免把宿主平台的 `gnu` / `musl` / `windows` / `macos` 选择交给上游默认猜测。
- `uv_python` 当请求 `3` 或 `3.13` 这类 family selector 时，会在所有匹配的托管解释器里选择版本最高的那个，不会因为目录字典序或旧安装残留而回退到更老的 patch 版本。
- `uv_python` 在失败回滚时会一起恢复同次安装里可能被改写的 `.uv-python`、`.uv-cache`、`.uv-bootstrap` 和 `managed_dir` 顶层 `python` / `python3` / `python3.x` shim；如果原目标本来不存在，也会清掉失败半程留下的残留，而不是把半更新状态留在托管根里。
- `system_package`、`pip`、`npm_global`、`workspace_package`、`cargo_install`、`rustup_component`、`go_install`，以及 bootstrap 为托管 `uv` 走的 `python -m pip install --target ... uv` fallback，都会通过统一 host recipe 执行边界施加 hard timeout；默认超时是 `900` 秒，也可通过 `--host-recipe-timeout-seconds` 或 `TOOLCHAIN_INSTALLER_HOST_RECIPE_TIMEOUT_SECONDS` 覆盖。超时会直接返回 install error，并附带有界 stdout/stderr，而不是无限期挂住整个 installer。
- `uv_python`、`uv_tool` 的托管 `uv` 安装子进程都会带 hard timeout，并对 stdout/stderr 做有界捕获；默认超时是 `900` 秒，也可通过 `TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS` 覆盖。installer 不会因为挂起进程或无限输出把自身卡死或无限占用内存。
- `uv_tool` 若目标路径上已有同名旧二进制，installer 会先把旧文件挪到临时备份；只有本次 `uv tool install` 真正产出新的目标二进制，且该入口还能通过一次带超时上限的 `--version` 健康探测后才算成功，失败时会恢复旧文件。
- `uv_tool` 在失败回滚时不只恢复最终 binary，也会恢复同次安装里可能被改写的 `.uv-tools`、`.uv-cache`、`.uv-bootstrap` 和共享 `.uv-python` 状态根，避免 launcher 恢复了但托管环境仍停在半更新状态。

## 来源探测与回退

- `uv_tool`
  - 当调用方没有显式提供任何包索引时，默认只使用官方 PyPI `https://pypi.org/simple`。
  - CLI `--package-index` 优先级高于 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES`；只有 CLI 没显式传索引时才读取环境变量。
  - 当调用方显式提供了 `--package-index` 或 `TOOLCHAIN_INSTALLER_PACKAGE_INDEXES` 时，installer 不再隐式把官方 PyPI 插到最前面；最终生效的显式索引顺序就是候选顺序。
  - 若显式索引、镜像或镜像前缀里出现重复值，只会保留第一次出现的位置，不会按字典序重排。
  - 安装前会探测显式索引的可达性，把可达源优先用于安装。
  - 结果里的 `source` 会对显式索引做脱敏，只保留协议、主机和路径，不回显 URL 中的用户信息、query 或 fragment。
  - 调用时会显式移除宿主进程继承的 `UV_*` 环境变量，只保留 installer 自己注入的托管目录布局和显式索引配置，避免外部 shell 状态静默污染来源选择。
  - `uv tool install` 子流程会带 hard timeout 和 bounded stdout/stderr capture 执行；默认超时是 `900` 秒，也可通过 `TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS` 覆盖。超时会直接返回 install error，而不是无限期挂住整个 installer。
- `uv_python`
  - CLI `--python-mirror` 优先级高于 `TOOLCHAIN_INSTALLER_PYTHON_INSTALL_MIRRORS`；只有 CLI 没显式传镜像时才读取环境变量。
  - 安装前会先对官方 Python 下载来源与最终生效的显式 mirror 做可达性探测；可达来源会被优先尝试，而官方来源在同等可达性下仍保持默认首位。
  - 备用镜像列表若有重复值，只保留第一次出现的位置。
  - 官方来源成功时，结果里的 `source_kind` 会是 `canonical`；只有显式镜像命中时才会是 `python_mirror`。
  - 结果里的 `source` 会对显式镜像做脱敏，只保留协议、主机和路径，不回显 URL 中的用户信息、query 或 fragment。
  - 调用时会显式移除宿主进程继承的 `UV_*` 环境变量，只保留 installer 自己注入的托管目录布局和显式 Python mirror 配置，避免外部 shell 状态静默污染来源选择。
  - `uv python install` 子流程会带 hard timeout 和 bounded stdout/stderr capture 执行；默认超时是 `900` 秒，也可通过 `TOOLCHAIN_INSTALLER_UV_TIMEOUT_SECONDS` 覆盖。超时会直接返回 install error，而不是无限期挂住整个 installer。
- `release`、`archive_tree_release`
  - 资产类型判断基于 URL 的 path 最后一段，不把 query string 当成资产名的一部分；`tool.tar.gz?download=1` 仍按 `tool.tar.gz` 处理。
  - 结果里的 `source` 会对最终命中的下载 URL 做脱敏，只保留协议、主机和路径，不回显用户信息、query 或 fragment。
- `release`
  - 通过内置来源规则、镜像前缀与可达性结果确定下载候选顺序。
  - CLI `--mirror-prefix` 优先级高于 `TOOLCHAIN_INSTALLER_MIRROR_PREFIXES`；只有 CLI 没显式传镜像前缀时才读取环境变量。
  - 生效镜像前缀里的重复值只去重，不重排显式顺序。

## 参考输入

- Python 工具链组合 plan：`../../examples/python-plan.json`
- 单独安装 `uv`：`../../examples/uv-plan.json`
- 单独安装 `ruff`：`../../examples/ruff-plan.json`
- 单独安装 `mypy`：`../../examples/mypy-plan.json`

这些文件是可执行参考，不应在文档里维护第二套不同版本的示例。
