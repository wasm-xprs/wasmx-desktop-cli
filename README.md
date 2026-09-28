# wasmx-desktop-cli

Rust CLI for the local wasm-xprs desktop control plane.

Commands:

- `status`
- `list`
- `deploy --tenant TENANT --deployment GENERATION --module ./function.wasm [--ores-adapter ./adapter.json]`
- `invoke --tenant TENANT --deployment GENERATION --payload JSON`
- `delete --tenant TENANT --deployment GENERATION`

The CLI talks only to `wasmx-desktop-daemon` over loopback by default and reads the daemon bearer token from `~/.wasm-xprs/daemon/token`.

Examples:

```sh
cargo run -- status

cargo run -- list
- inspect --tenant TENANT --deployment GENERATION

cargo run -- deploy \
  --tenant local-dev \
  --deployment echo-v1 \
  --module ./echo.wasm

cargo run -- deploy \
  --tenant local-dev \
  --deployment ores-echo-v1 \
  --module ./echo.wasm \
  --ores-adapter ./lambda-adapter.wasm-xprs.v1.json

cargo run -- invoke \
  --tenant local-dev \
  --deployment echo-v1 \
  --payload '{"hello":"world"}'
```

`--ores-adapter` accepts an `ores.lambda.adapter/v1` JSON descriptor emitted by `ores-stack`. The CLI only checks that the file is a bounded JSON object and transmits it unchanged; the daemon remains the semantic authority and rejects descriptors that do not bind to `wasm_xprs`, `wasm32-unknown-unknown`, `wasmx-v1`, `wasmtime_store`, and `wasi_enabled=false`.

Use `--url` to override the daemon URL and `--timeout` to bound invocation wall time. Plain HTTP daemon URLs are accepted only for loopback (`127.0.0.1`, `::1`, or `localhost`); remote daemon URLs must use HTTPS so the bearer token is not transmitted in cleartext.

## Transport hardening

The CLI refuses redirects for authenticated daemon requests, rejects plaintext HTTP for non-loopback hosts, validates deployment identifiers before constructing URL paths, bounds daemon responses, and on Unix refuses token files readable by group or other users.
