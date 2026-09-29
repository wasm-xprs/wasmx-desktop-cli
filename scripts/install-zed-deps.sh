#!/usr/bin/env bash
set -euo pipefail

readonly ZED_CLI_REV="${ZED_CLI_REV:-b23ae010f8c0837e0251302876ec9a087f965647}"

if ! command -v zed >/dev/null 2>&1; then
  cargo install \
    --git https://github.com/zed-pkg/zed-cli.git \
    --rev "${ZED_CLI_REV}" \
    --locked \
    zed-cli
fi

zed install

test -f .vendor/.zed/oresoftware/ores-clis-core/Cargo.toml
