# 安装示例

以下示例覆盖直接参数模式与 plan 模式。更完整的 Python 3.13.12 工具链流程见 `python-toolchain-bootstrap.md`。

## Node.js

```bash
NODE_VER="$(curl -fsSL https://nodejs.org/dist/index.json | jq -r '[.[] | select(.lts != false)][0].version')"
NODE_URL="https://nodejs.org/dist/${NODE_VER}/node-${NODE_VER}-linux-x64.tar.xz"
toolchain-installer bootstrap --json \
  --method release \
  --id node \
  --url "${NODE_URL}" \
  --binary-name node \
  --archive-binary "node-${NODE_VER}-linux-x64/bin/node"
```

## Go

```bash
GO_VER="$(curl -fsSL 'https://go.dev/dl/?mode=json' | jq -r '[.[] | select(.stable == true)][0].version')"
GO_URL="https://go.dev/dl/${GO_VER}.linux-amd64.tar.gz"
toolchain-installer bootstrap --json \
  --method release \
  --id go \
  --url "${GO_URL}" \
  --binary-name go \
  --archive-binary "go/bin/go"
```

## uv

```bash
toolchain-installer bootstrap --json \
  --method uv \
  --id uv
```

参考 plan：`../../examples/uv-plan.json`

## Rust

```bash
RUSTUP_JSON="$(toolchain-installer bootstrap --json \
  --method release \
  --id rustup-init \
  --url "https://github.com/rust-lang/rustup/releases/latest/download/rustup-init-x86_64-unknown-linux-gnu" \
  --binary-name rustup-init)"

RUSTUP_BIN="$(echo "${RUSTUP_JSON}" | jq -r '.items[0].destination')"
"${RUSTUP_BIN}" -y --default-toolchain stable
```

## 示例 plan 索引

- `../../examples/nodejs-plan.json`
- `../../examples/go-plan.json`
- `../../examples/rust-plan.json`
- `../../examples/python-plan.json`

如果只是查 plan 文件职责，请看 `../references/example-plan-files.md`。
