mod ores_evidence;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flags2env::BundledFlags2Env;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, env, net::IpAddr, path::PathBuf, time::Duration};
use uuid::Uuid;

const MAX_MODULE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ORES_ADAPTER_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_FUEL: u64 = 500_000_000;

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct CliConfig {
    WASMX_DESKTOP_DAEMON_URL: String,
    WASMX_DESKTOP_TIMEOUT_MS: i64,
    WASMX_DESKTOP_TENANT_ID: Option<String>,
    WASMX_DESKTOP_DEPLOYMENT_ID: Option<String>,
    WASMX_DESKTOP_MODULE: Option<String>,
    WASMX_DESKTOP_ORES_ADAPTER: Option<String>,
    WASMX_DESKTOP_ORES_RECEIPT: Option<String>,
    WASMX_DESKTOP_PAYLOAD: Option<Value>,
    WASMX_DESKTOP_FUEL: Option<i64>,
    WASMX_DESKTOP_INVOCATION_ID: Option<String>,
    FLAGS2ENV_COMMAND: Option<String>,
}

#[tokio::main]
async fn main() {
    let code = match run().await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("wasmx-desktop-cli: {error}");
            2
        }
    };
    std::process::exit(code);
}

async fn run() -> Result<()> {
    let config_path = resolve_config_path()?;
    let config_path_text = config_path
        .to_str()
        .ok_or_else(|| anyhow!(".cli-flags.toml path is not UTF-8"))?;
    let parser = BundledFlags2Env::new();
    parser
        .audit_config(Some(config_path_text))
        .map_err(|error| anyhow!(error.to_string()))?;

    let argv = env::args().collect::<Vec<_>>();
    let parsed = parser
        .parse_structured(&argv, Some(config_path_text))
        .map_err(|error| anyhow!(error.to_string()))?;
    if !parsed.unknown_options.is_empty() {
        bail!(
            "unknown command-line options: {}",
            parsed.unknown_options.len()
        );
    }
    if !parsed.errors.is_empty() {
        bail!("invalid command-line values: {}", parsed.errors.join("; "));
    }
    if !parsed.extras.is_empty() {
        bail!("unexpected positional arguments: {}", parsed.extras.len());
    }

    let mut raw = env::vars().collect::<HashMap<_, _>>();
    raw.remove("FLAGS2ENV_COMMAND");
    raw.extend(parsed.provided_flags);
    let config = parser
        .coerce::<CliConfig, _>(&raw, Some(config_path_text))
        .map_err(|error| anyhow!(error.to_string()))?;

    let command = config.FLAGS2ENV_COMMAND.as_deref().unwrap_or("");
    let timeout_ms = u64::try_from(config.WASMX_DESKTOP_TIMEOUT_MS)
        .ok()
        .filter(|value| *value > 0 && *value <= 1_200_000)
        .ok_or_else(|| anyhow!("--timeout must be between 1 and 1200000 ms"))?;
    let token = read_token()?;
    let base_url = validate_daemon_url(&config.WASMX_DESKTOP_DAEMON_URL)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(timeout_ms.saturating_add(5_000)))
        .build()?;

    match command {
        "status" => {
            let response = client
                .get(format!("{base_url}/v1/status"))
                .bearer_auth(&token)
                .send()
                .await?;
            print_json_response(response).await?;
        }
        "doctor" => {
            let ready = client.get(format!("{base_url}/readyz")).send().await?;
            if !ready.status().is_success() {
                bail!("daemon readiness check failed with {}", ready.status());
            }

            let response = client
                .get(format!("{base_url}/v1/status"))
                .bearer_auth(&token)
                .send()
                .await?;
            let status = read_json_response(response).await?;
            validate_status_contract(&status)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ready": true,
                    "runtime_contract_verified": true,
                    "status": status,
                }))?
            );
        }
        "list" => {
            let response = client
                .get(format!("{base_url}/v1/deployments"))
                .bearer_auth(&token)
                .send()
                .await?;
            print_json_response(response).await?;
        }
        "inspect" => {
            let tenant_id = validated_id(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = validated_id(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
            let response = client
                .get(format!(
                    "{base_url}/v1/deployments/{tenant_id}/{deployment_id}"
                ))
                .bearer_auth(&token)
                .send()
                .await?;
            print_json_response(response).await?;
        }
        "deploy" => {
            let tenant_id = validated_id(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = validated_id(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
            let module_path = required(config.WASMX_DESKTOP_MODULE, "--module")?;
            let bytes = tokio::fs::read(&module_path)
                .await
                .with_context(|| format!("cannot read module {module_path}"))?;
            if bytes.is_empty() || bytes.len() > MAX_MODULE_BYTES {
                bail!("module must be between 1 and {MAX_MODULE_BYTES} bytes");
            }

            let ores_adapter_path = config.WASMX_DESKTOP_ORES_ADAPTER.as_deref();
            let ores_adapter = read_optional_ores_adapter(ores_adapter_path).await?;
            let adapter_supplied = ores_adapter.is_some();
            let ores_evidence = read_optional_ores_evidence(
                ores_adapter_path,
                config.WASMX_DESKTOP_ORES_RECEIPT.as_deref(),
                &bytes,
            )
            .await?;
            let receipt_supplied = ores_evidence.is_some();
            let mut body = json!({
                "tenant_id": tenant_id,
                "deployment_id": deployment_id,
                "wasm_base64": BASE64.encode(&bytes),
            });
            if let Some(adapter) = ores_adapter {
                body["ores_adapter"] = adapter;
            }
            if let Some(evidence) = ores_evidence {
                body["ores_adapter_raw_base64"] =
                    Value::String(BASE64.encode(&evidence.adapter_bytes));
                body["ores_receipt_raw_base64"] =
                    Value::String(BASE64.encode(&evidence.receipt_bytes));
            }

            let expected_sha256 = format!("{:x}", Sha256::digest(&bytes));
            let response = client
                .post(format!("{base_url}/v1/deploy"))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;
            let value = read_json_response(response).await?;
            validate_deploy_ack(
                &value,
                &tenant_id,
                &deployment_id,
                &expected_sha256,
                bytes.len(),
                adapter_supplied,
            )?;
            validate_receipt_ack(&value, receipt_supplied)?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        "invoke" => {
            let tenant_id = validated_id(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = validated_id(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
            let payload_json = config.WASMX_DESKTOP_PAYLOAD.unwrap_or_else(|| json!({}));
            let fuel = config
                .WASMX_DESKTOP_FUEL
                .map(|value| {
                    return u64::try_from(value)
                        .ok()
                        .filter(|fuel| *fuel > 0 && *fuel <= MAX_FUEL)
                        .ok_or_else(|| anyhow!("--fuel must be between 1 and {MAX_FUEL}"));
                })
                .transpose()?;
            let invocation_id = match config.WASMX_DESKTOP_INVOCATION_ID {
                Some(value) => validated_id(Some(value), "--invocation-id")?,
                None => Uuid::new_v4().to_string(),
            };
            let body = json!({
                "invocation_id": invocation_id,
                "tenant_id": tenant_id,
                "deployment_id": deployment_id,
                "payload_json": payload_json,
                "timeout_ms": timeout_ms,
                "fuel": fuel,
            });
            let response = client
                .post(format!("{base_url}/v1/invoke"))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;
            print_json_response(response).await?;
        }
        "delete" => {
            let tenant_id = validated_id(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = validated_id(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
            let response = client
                .delete(format!(
                    "{base_url}/v1/deployments/{tenant_id}/{deployment_id}"
                ))
                .bearer_auth(&token)
                .send()
                .await?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await?;
                bail!("daemon returned {status}: {body}");
            }
            println!("deleted {tenant_id}/{deployment_id}");
        }
        _ => {
            bail!("command required: status, doctor, list, inspect, deploy, invoke or delete");
        }
    }

    return Ok(());
}

async fn read_optional_ores_adapter(path: Option<&str>) -> Result<Option<Value>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if path.trim().is_empty() {
        bail!("--ores-adapter must name a readable JSON file");
    }
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("cannot read ORES adapter {path}"))?;
    return parse_ores_adapter_bytes(&bytes).map(Some);
}

async fn read_optional_ores_evidence(
    adapter_path: Option<&str>,
    receipt_path: Option<&str>,
    module_bytes: &[u8],
) -> Result<Option<ores_evidence::CheckedOresEvidence>> {
    let Some(receipt_path) = receipt_path else {
        return Ok(None);
    };
    let adapter_path = adapter_path
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("--ores-receipt requires --ores-adapter"))?;
    if receipt_path.trim().is_empty() {
        bail!("--ores-receipt must name a readable JSON file");
    }
    let adapter_bytes = tokio::fs::read(adapter_path)
        .await
        .with_context(|| format!("cannot read ORES adapter {adapter_path}"))?;
    if adapter_bytes.is_empty() || adapter_bytes.len() > MAX_ORES_ADAPTER_BYTES {
        bail!("ORES adapter must be between 1 and {MAX_ORES_ADAPTER_BYTES} bytes");
    }
    let receipt_bytes = tokio::fs::read(receipt_path)
        .await
        .with_context(|| format!("cannot read ORES receipt {receipt_path}"))?;
    let evidence = ores_evidence::validate(
        adapter_bytes,
        receipt_bytes,
        module_bytes,
        "wasm_xprs",
        "wasm32-unknown-unknown",
    )?;
    return Ok(Some(evidence));
}

fn parse_ores_adapter_bytes(bytes: &[u8]) -> Result<Value> {
    if bytes.is_empty() || bytes.len() > MAX_ORES_ADAPTER_BYTES {
        bail!("ORES adapter must be between 1 and {MAX_ORES_ADAPTER_BYTES} bytes");
    }
    let value: Value = serde_json::from_slice(bytes).context("ORES adapter is not valid JSON")?;
    if !value.is_object() {
        bail!("ORES adapter must be a JSON object");
    }
    return Ok(value);
}

async fn read_json_response(mut response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        bail!("daemon response exceeds {MAX_RESPONSE_BYTES} bytes");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            bail!("daemon response exceeds {MAX_RESPONSE_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }

    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        bail!("daemon returned {status}: {text}");
    }
    let value: Value = serde_json::from_slice(&body).context("daemon response was not JSON")?;
    Ok(value)
}

async fn print_json_response(response: reqwest::Response) -> Result<()> {
    let value = read_json_response(response).await?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn validate_deploy_ack(
    value: &Value,
    tenant_id: &str,
    deployment_id: &str,
    expected_sha256: &str,
    expected_module_bytes: usize,
    adapter_supplied: bool,
) -> Result<()> {
    if value.get("tenant_id").and_then(Value::as_str) != Some(tenant_id) {
        bail!("daemon deploy response tenant_id did not match the requested tenant");
    }
    if value.get("deployment_id").and_then(Value::as_str) != Some(deployment_id) {
        bail!("daemon deploy response deployment_id did not match the requested deployment");
    }
    if value.get("sha256").and_then(Value::as_str) != Some(expected_sha256) {
        bail!("daemon deploy response sha256 did not match the uploaded module");
    }
    let expected_module_bytes = u64::try_from(expected_module_bytes)
        .map_err(|_| anyhow!("module byte length does not fit in u64"))?;
    if value.get("module_bytes").and_then(Value::as_u64) != Some(expected_module_bytes) {
        bail!("daemon deploy response module_bytes did not match the uploaded module");
    }
    if value.get("compiled").and_then(Value::as_bool) != Some(true) {
        bail!("daemon deploy response did not confirm compilation");
    }
    if value.get("ores_adapter_verified").and_then(Value::as_bool) != Some(adapter_supplied) {
        bail!(
            "daemon deploy response adapter verification did not match whether an ORES adapter was supplied"
        );
    }
    return Ok(());
}

fn validate_receipt_ack(value: &Value, receipt_supplied: bool) -> Result<()> {
    if value.get("ores_receipt_verified").and_then(Value::as_bool) != Some(receipt_supplied) {
        bail!(
            "daemon deploy response receipt verification did not match whether ORES receipt evidence was supplied"
        );
    }
    return Ok(());
}

fn validate_status_contract(value: &Value) -> Result<()> {
    let expected = [
        ("runtime", Value::String("wasmtime".to_owned())),
        (
            "isolation",
            Value::String("fresh_store_per_invocation".to_owned()),
        ),
        ("guest_abi", Value::String("wasmx-v1".to_owned())),
        (
            "target_triple",
            Value::String("wasm32-unknown-unknown".to_owned()),
        ),
        ("wasi_enabled", Value::Bool(false)),
        ("store_per_invocation", Value::Bool(true)),
    ];
    for (field, expected_value) in expected {
        if value.get(field) != Some(&expected_value) {
            bail!("daemon runtime contract mismatch for field {field}");
        }
    }
    Ok(())
}

fn required(value: Option<String>, flag: &str) -> Result<String> {
    return value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{flag} is required"));
}

fn validated_id(value: Option<String>, flag: &str) -> Result<String> {
    let value = required(value, flag)?;
    let valid = value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && value != "."
        && value != "..";
    if !valid {
        bail!("{flag} must contain only ASCII letters, digits, '.', '_' or '-'");
    }
    Ok(value)
}

fn validate_daemon_url(value: &str) -> Result<String> {
    let url = reqwest::Url::parse(value).context("daemon URL is invalid")?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("daemon URL must not embed credentials");
    }
    if url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        bail!("daemon URL must be an origin without a path, query, or fragment");
    }
    if !matches!(url.scheme(), "http" | "https") {
        bail!("daemon URL scheme must be http or https");
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("daemon URL must contain a host"))?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    if url.scheme() == "http" && !loopback {
        bail!("plain HTTP daemon URLs are allowed only for loopback; use HTTPS remotely");
    }

    return Ok(url.as_str().trim_end_matches('/').to_owned());
}

fn resolve_config_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WASMX_DESKTOP_FLAGS_CONFIG") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!("WASMX_DESKTOP_FLAGS_CONFIG is not a readable file");
    }

    let current = env::current_dir()?.join(".cli-flags.toml");
    if current.is_file() {
        return Ok(current);
    }

    let executable = env::current_exe()?;
    if let Some(parent) = executable.parent() {
        let adjacent = parent.join(".cli-flags.toml");
        if adjacent.is_file() {
            return Ok(adjacent);
        }
    }

    bail!("cannot locate .cli-flags.toml")
}

fn read_token() -> Result<String> {
    let path = if let Some(path) = env::var_os("WASMX_DESKTOP_TOKEN_FILE") {
        PathBuf::from(path)
    } else {
        let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is required"))?;
        PathBuf::from(home).join(".wasm-xprs/daemon/token")
    };
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("cannot inspect daemon token at {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("daemon token path must be a regular non-symlink file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("daemon token file must not be accessible by group or other users");
        }
    }
    let token = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read daemon token at {}", path.display()))?;
    let token = token.trim();
    if token.len() < 32 {
        bail!("daemon token is invalid");
    }
    return Ok(token.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_rejects_wrong_runtime_contract() -> Result<()> {
        let good = json!({
            "runtime": "wasmtime",
            "isolation": "fresh_store_per_invocation",
            "guest_abi": "wasmx-v1",
            "target_triple": "wasm32-unknown-unknown",
            "wasi_enabled": false,
            "store_per_invocation": true
        });
        validate_status_contract(&good)?;

        let mut bad = good;
        bad["wasi_enabled"] = Value::Bool(true);
        assert!(validate_status_contract(&bad).is_err());
        Ok(())
    }

    #[test]
    fn deploy_ack_binds_exact_requested_identity() -> Result<()> {
        let expected_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let good = json!({
            "tenant_id": "tenant-a",
            "deployment_id": "release-1",
            "sha256": expected_sha256,
            "module_bytes": 42,
            "compiled": true,
            "ores_adapter_verified": true
        });
        validate_deploy_ack(&good, "tenant-a", "release-1", expected_sha256, 42, true)?;

        for field in ["tenant_id", "deployment_id", "sha256"] {
            let mut bad = good.clone();
            bad[field] = Value::String("wrong".to_owned());
            assert!(
                validate_deploy_ack(&bad, "tenant-a", "release-1", expected_sha256, 42, true,)
                    .is_err()
            );
        }

        let mut bad = good.clone();
        bad["module_bytes"] = Value::from(41_u64);
        assert!(
            validate_deploy_ack(&bad, "tenant-a", "release-1", expected_sha256, 42, true,).is_err()
        );

        let mut bad = good.clone();
        bad["compiled"] = Value::Bool(false);
        assert!(
            validate_deploy_ack(&bad, "tenant-a", "release-1", expected_sha256, 42, true,).is_err()
        );

        let mut bad = good;
        bad["ores_adapter_verified"] = Value::Bool(false);
        assert!(
            validate_deploy_ack(&bad, "tenant-a", "release-1", expected_sha256, 42, true,).is_err()
        );
        return Ok(());
    }

    #[test]
    fn deploy_ack_without_adapter_requires_unverified_status() -> Result<()> {
        let expected_sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let value = json!({
            "tenant_id": "tenant-a",
            "deployment_id": "release-2",
            "sha256": expected_sha256,
            "module_bytes": 7,
            "compiled": true,
            "ores_adapter_verified": false
        });
        return validate_deploy_ack(&value, "tenant-a", "release-2", expected_sha256, 7, false);
    }

    #[test]
    fn receipt_ack_requires_exact_verification_status() -> Result<()> {
        validate_receipt_ack(&json!({"ores_receipt_verified": true}), true)?;
        validate_receipt_ack(&json!({"ores_receipt_verified": false}), false)?;
        assert!(validate_receipt_ack(&json!({"ores_receipt_verified": false}), true).is_err());
        assert!(validate_receipt_ack(&json!({}), false).is_err());
        return Ok(());
    }

    #[test]
    fn daemon_url_rejects_remote_plaintext() -> Result<()> {
        assert_eq!(
            validate_daemon_url("http://127.0.0.1:8765")?,
            "http://127.0.0.1:8765"
        );
        assert_eq!(
            validate_daemon_url("http://localhost:8765")?,
            "http://localhost:8765"
        );
        assert!(validate_daemon_url("http://example.com:8765").is_err());
        assert!(validate_daemon_url("https://example.com/api").is_err());
        assert!(validate_daemon_url("ftp://127.0.0.1").is_err());
        return Ok(());
    }

    #[test]
    fn identifiers_are_restricted_before_path_interpolation() -> Result<()> {
        assert_eq!(
            validated_id(Some("tenant-1.alpha".to_owned()), "--tenant")?,
            "tenant-1.alpha"
        );
        assert!(validated_id(Some("../escape".to_owned()), "--tenant").is_err());
        assert!(validated_id(Some("a/b".to_owned()), "--tenant").is_err());
        return Ok(());
    }

    #[test]
    fn sha256_is_stable_for_uploaded_module_bytes() {
        let digest = format!("{:x}", Sha256::digest(b"wasmx"));
        assert_eq!(
            digest,
            "1d47fb365312446e2c62ef0e85f757af3c123f88cb1ca846c36bf7ebae57ee9d"
        );
    }

    #[test]
    fn ores_adapter_must_be_a_bounded_json_object() -> Result<()> {
        let adapter = parse_ores_adapter_bytes(
            br#"{"schema_version":"ores.lambda.adapter/v1","provider":"wasm_xprs"}"#,
        )?;
        assert_eq!(adapter["provider"], "wasm_xprs");
        assert!(parse_ores_adapter_bytes(b"[]").is_err());
        assert!(parse_ores_adapter_bytes(b"").is_err());
        return Ok(());
    }
}
