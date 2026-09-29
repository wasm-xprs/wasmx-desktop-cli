#!/usr/bin/env bash
set -euo pipefail

readonly ZED_VERSION="0.2.3"
readonly ZED_CLI_REV="${ZED_CLI_REV:-b23ae010f8c0837e0251302876ec9a087f965647}"

install_released_zed() {
  local os arch asset sha tmp zed_bin
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}:${arch}" in
    Darwin:arm64)
      asset="zed-aarch64-apple-darwin.tar.gz"
      sha="46052288d8a5ca178e7f942aeb03197022416ce13c3ac4ebd0fea66963977e62"
      ;;
    Linux:x86_64)
      asset="zed-x86_64-unknown-linux-gnu.tar.gz"
      sha="b8f81a8e4943cbaeb47153386819a911dcf02666a8709c5bd013e4f35b1ba1b9"
      ;;
    *)
      return 1
      ;;
  esac

  tmp="$(mktemp -d)"
  trap 'rm -rf "${tmp:-}"' RETURN
  curl --fail --location --retry 3 \
    --output "$tmp/$asset" \
    "https://github.com/zed-pkg/zed-cli/releases/download/v${ZED_VERSION}/${asset}"

  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "$sha" "$tmp/$asset" | sha256sum --check --strict
  else
    printf '%s  %s\n' "$sha" "$tmp/$asset" | shasum -a 256 --check
  fi

  tar -xzf "$tmp/$asset" -C "$tmp"
  zed_bin="$tmp/zed"
  test -x "$zed_bin"
  "$zed_bin" install
}

if command -v zed >/dev/null 2>&1; then
  zed install
elif ! install_released_zed; then
  cargo install \
    --git https://github.com/zed-pkg/zed-cli.git \
    --rev "$ZED_CLI_REV" \
    --locked \
    zed-cli
  zed install
fi

test -f .vendor/.zed/oresoftware/ores-clis-core/Cargo.toml
