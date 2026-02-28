# Worker 约束

- 仅支持 `GET/HEAD`。
- 仅支持路由：`/toolchain/git/{tag}/{asset}`。
- 仅允许 `cf-ipcountry=CN` 或 `request.cf.country=CN`。
- 仅重定向到 `git-for-windows` 官方 release 资产。
- 不支持 `?url=` 任意代理参数。

本目录可直接执行：

```bash
npm test
```
