#!/usr/bin/env bash
# Installs the Rust toolchain and the Dioxus CLI on Vercel's build image (Amazon Linux).
set -euo pipefail

DX_VERSION="0.7.10"
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi
rustup target add wasm32-unknown-unknown

if ! command -v dx >/dev/null 2>&1 || [ "$(dx --version | awk '{print $2}')" != "$DX_VERSION" ]; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    curl -L --proto '=https' --tlsv1.2 -sSf \
      https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
  fi
  cargo binstall "dioxus-cli@${DX_VERSION}" --no-confirm --force
fi
dx --version
