# 安全边界

## 威胁模型

- 把 Worker 滥用为开放代理转发任意流量。
- 高频请求消耗 Worker 配额或触发上游限流。
- 篡改下载内容导致执行恶意二进制。
- 通过异常参数触发 SSRF 或白名单逃逸。
- 通过 `destination` 等路径字段写出预期目录之外。

## 强制策略

1. 固定路由
   - 仅允许 `/toolchain/git/{tag}/{asset}`，禁止任何查询参数。
2. 白名单来源
   - 只允许预定义域名与资产模式，非白名单直接拒绝。
3. 中国流量限定
   - Worker 仅允许 `CF-IPCountry=CN` 的请求，其他国家或地区直接拒绝。
4. `git release` 限定
   - Worker 仅服务 `git release` 资产路由，不代理 `gh` 或其他工具下载。
5. 默认重定向
   - 网关优先返回 `302/307`，不转发大文件内容。
6. 限流
   - 按 IP 与路径限流，超过阈值返回 `429`。
   - 当前 Worker 实现为单实例内存限流；若对公网长期暴露，应替换为集中式限流。
7. 完整性校验
   - 下载后必须执行哈希校验；校验失败直接终止安装。
8. 最小日志
   - 只记录必要诊断字段，避免泄露凭证或敏感头。
   - `uv_tool` / `uv_python` 对外结果里的 `source` 不回显显式索引或镜像 URL 中的用户信息、query 或 fragment。
9. 路径约束
   - Unix 风格绝对路径如 `/tmp/demo` 会直接拒绝，不能借此绕过托管目录。
   - Windows 绝对路径如 `C:\tools\demo.exe` 只在 Windows 宿主上按原样保留；非 Windows 宿主会直接拒绝，避免把 Windows-local 语法误当成相对路径落盘。
   - plan 中的 `destination` 禁止包含 `..`。
   - 同时拒绝 Windows drive-relative 路径如 `C:foo`，以及 Windows root-relative 路径如 `\foo`。
   - `id` 与 `binary_name` 都必须是纯叶子名，不允许把路径片段塞进默认目标名。
   - `pip`、`npm_global`、`workspace_package`、`cargo_install`、`go_install`、`rustup_component`、`uv_tool` 的声明式 `package` 字段不能长得像命令行选项；`--editable`、`--workspace`、`--git`、`--toolchain`、`--index-url` 这类值会在 resolve 阶段直接拒绝。
   - 会按本地路径解释的 `package` 只接受当前宿主机原生的绝对路径语法；非 Windows 宿主会直接拒绝 `C:\repo\demo` 或 `file:C:\repo\demo` 这类 Windows-local 路径，避免把伪绝对路径落成相对文件操作。
   - `pip` 的默认解释器回退只会发生在命令缺失时；如果首选解释器已经真实执行安装并失败，installer 会直接返回失败，避免同一 plan 被静默装进另一个 Python 环境。
   - 相对路径只会解析到 `managed_dir` 下；当目标是 Windows 时，`bin\\tool.exe` 与 `bin/tool.exe` 会按同一相对层级处理。
   - 解析后若两个 item 指向同一目标路径，或一个目标路径嵌套在另一个目标路径之下，plan 会在执行前直接拒绝。
   - `npm_global` 允许复用托管目录里已有的 leaf symlink 入口，但这个 symlink 解析后的目标仍必须留在 `managed_dir` 内；不能借复用入口把实际执行路径指到托管根外部。
10. 托管 bootstrap 健康检查
   - 托管安装复用只适用于内置受支持工具；未知工具即使在 `managed_dir` 下存在同名且可执行的文件，也不会被误报成 `installed`。
   - Windows managed `git` 的 launcher 只允许指向 `managed_dir` 内的 MinGit payload；带 `..` 的逃逸路径会被直接视为损坏安装。
   - Windows managed `git` 的 launcher 还必须落在 `managed_dir/git-portable/PortableGit/` 这棵 payload 子树内；即使仍在 `git-portable/` 下，只要指向其他旁路目录也会被视为损坏安装。
   - Windows managed `git` 不会只凭 launcher、`git.exe` 和 DLL 文件存在就被视为健康；installer 还会执行 payload 的 `--version` 健康检查。
   - `bootstrap` 只有在宿主机上解析到可实际执行的命令后才会返回 `present`；PATH 中同名但不可执行的普通文件不会阻止后续安装。

## 验收测试

- 请求 `?url=https://example.com/a` 返回拒绝。
- 非中国流量请求 Worker 返回拒绝。
- `gh` 相关路由请求 Worker 返回拒绝。
- 非白名单域名候选全部被拒绝。
- 同一 IP 与路径连续超阈值请求后出现 `429`。
- 人工篡改下载文件时，校验失败且不写入目标路径。

## 边界说明

- 主 CLI 在没有 Worker 的情况下也必须能直接走公共来源工作。
- Worker 是可选网络优化层，不是正确性的唯一来源。
- installer 与外部网关的集成边界见 `external-gateway-integration.md`。
- 外部网关自身的具体路由、方法与限流行为见 `../../../toolchain-edge-gateway/docs/operations/request-routing-and-rate-limits.md`。
