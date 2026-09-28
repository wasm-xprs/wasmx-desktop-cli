use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flags2env::BundledFlags2Env;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, env, path::PathBuf, time::Duration};
use uuid::Uuid;

const MAX_MODULE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ORES_ADAPTER_BYTES: usize = 1024 * 1024;

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct CliConfig {
    WASMX_DESKTOP_DAEMON_URL: String,
    WASMX_DESKTOP_TIMEOUT_MS: i64,
    WASMX_DESKTOP_TENANT_ID: Option<String>,
    WASMX_DESKTOP_DEPLOYMENT_ID: Option<String>,
    WASMX_DESKTOP_MODULE: Option<String>,
    WASMX_DESKTOP_ORES_ADAPTER: Option<String>,
    WASMX_DESKTOP_PAYLOAD: Option<Value>,
    WASMX_DESKTOP_FUEL: Option<i64>,
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
        bail!(
            "unexpected positional arguments: {}",
            parsed.extras.len()
        );
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
    let base_url = trim_url(&config.WASMX_DESKTOP_DAEMON_URL);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(
            timeout_ms.saturating_add(5_000),
        ))
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
        "list" => {
            let response = client
                .get(format!("{base_url}/v1/deployments"))
                .bearer_auth(&token)
                .send()
                .await?;
            print_json_response(response).await?;
        }
        "deploy" => {
            let tenant_id = required(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = required(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
            let module_path = required(config.WASMX_DESKTOP_MODULE, "--module")?;
            let bytes = tokio::fs::read(&module_path)
                .await
                .with_context(|| format!("cannot read module {module_path}"))?;
            if bytes.is_empty() || bytes.len() > MAX_MODULE_BYTES {
                bail!("module must be between 1 and {MAX_MODULE_BYTES} bytes");
            }

            let ores_adapter = read_optional_ores_adapter(
                config.WASMX_DESKTOP_ORES_ADAPTER.as_deref(),
            )
            .await?;
            let mut body = json!({
                "tenant_id": tenant_id,
                "deployment_id": deployment_id,
                "wasm_base64": BASE64.encode(bytes),
            });
            if let Some(adapter) = ores_adapter {
                body["ores_adapter"] = adapter;
            }

            let response = client
                .post(format!("{base_url}/v1/deploy"))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;
            print_json_response(response).await?;
        }
        "invoke" => {
            let tenant_id = required(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = required(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
            let payload_json = config.WASMX_DESKTOP_PAYLOAD.unwrap_or_else(|| json!({}));
            let fuel = config
                .WASMX_DESKTOP_FUEL
                .map(|value| {
                    return u64::try_from(value)
                        .ok()
                        .filter(|fuel| *fuel > 0)
                        .ok_or_else(|| anyhow!("--fuel must be greater than zero"));
                })
                .transpose()?;
            let body = json!({
                "invocation_id": Uuid::new_v4().to_string(),
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
            let tenant_id = required(config.WASMX_DESKTOP_TENANT_ID, "--tenant")?;
            let deployment_id = required(config.WASMX_DESKTOP_DEPLOYMENT_ID, "--deployment")?;
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
            bail!("command required: status, list, deploy, invoke or delete");
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

fn parse_ores_adapter_bytes(bytes: &[u8]) -> Result<Value> {
    if bytes.is_empty() || bytes.len() > MAX_ORES_ADAPTER_BYTES {
        bail!(
            "ORES adapter must be between 1 and {MAX_ORES_ADAPTER_BYTES} bytes"
        );
    }
    let value: Value = serde_json::from_slice(bytes).context("ORES adapter is not valid JSON")?;
    if !value.is_object() {
        bail!("ORES adapter must be a JSON object");
    }
    return Ok(value);
}

async fn print_json_response(response: reqwest::Response) -> Result<()> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!("daemon returned {status}: {body}");
    }
    let value: Value = serde_json::from_str(&body).context("daemon response was not JSON")?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    return Ok(());
}

fn required(value: Option<String>, flag: &str) -> Result<String> {
    return value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{flag} is required"));
}

fn trim_url(value: &str) -> &str {
    return value.trim_end_matches('/');
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
