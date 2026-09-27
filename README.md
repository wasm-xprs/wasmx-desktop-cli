# wasmx-desktop-cli

Rust CLI for the local wasm-xprs desktop control plane.

Commands:

- status
- deploy --tenant TENANT --deployment GENERATION --module ./function.wasm
- invoke --tenant TENANT --deployment GENERATION --payload JSON
- delete --tenant TENANT --deployment GENERATION

The CLI talks only to wasmx-desktop-daemon over loopback by default and reads the daemon bearer token from ~/.wasm-xprs/daemon/token.

Examples:

    cargo run -- status

    cargo run -- deploy \
      --tenant local-dev \
      --deployment echo-v1 \
      --module ./echo.wasm

    cargo run -- invoke \
      --tenant local-dev \
      --deployment echo-v1 \
      --payload '{"hello":"world"}'

Use --url to override the daemon URL and --timeout to bound invocation wall time.
