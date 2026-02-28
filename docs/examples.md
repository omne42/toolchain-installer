# 安装示例（无需 JSON）

以下示例都使用“直接参数模式”，调用方无需编写 JSON。

## Node.js（最新 LTS 稳定版）

```bash
NODE_VER="$(curl -fsSL https://nodejs.org/dist/index.json | jq -r '[.[] | select(.lts != false)][0].version')"
NODE_URL="https://nodejs.org/dist/${NODE_VER}/node-${NODE_VER}-linux-x64.tar.xz"
toolchain-installer --json \
  --method release \
  --id node \
  --url "${NODE_URL}" \
  --binary-name node \
  --archive-binary "node-${NODE_VER}-linux-x64/bin/node"
```

## Go（最新稳定版）

```bash
GO_VER="$(curl -fsSL 'https://go.dev/dl/?mode=json' | jq -r '[.[] | select(.stable == true)][0].version')"
GO_URL="https://go.dev/dl/${GO_VER}.linux-amd64.tar.gz"
toolchain-installer --json \
  --method release \
  --id go \
  --url "${GO_URL}" \
  --binary-name go \
  --archive-binary "go/bin/go"
```

## Python（3.13 最新稳定补丁）

```bash
toolchain-installer --json --method pip --id uv --package uv --python python3
uv python install 3.13
```

## ruff（最新稳定版）

```bash
toolchain-installer --json --method pip --id ruff --package ruff --python python3
```

## uv（最新稳定版）

```bash
toolchain-installer --json --method pip --id uv --package uv --python python3
```

## Rust（最新稳定通道）

```bash
RUSTUP_JSON="$(toolchain-installer --json \
  --method release \
  --id rustup-init \
  --url "https://github.com/rust-lang/rustup/releases/latest/download/rustup-init-x86_64-unknown-linux-gnu" \
  --binary-name rustup-init)"

RUSTUP_BIN="$(echo "${RUSTUP_JSON}" | jq -r '.items[0].destination')"
"${RUSTUP_BIN}" -y --default-toolchain stable
```
