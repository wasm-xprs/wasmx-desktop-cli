from pathlib import Path

path = Path('src/main.rs')
text = path.read_text()

replacements = [
    (
        '''struct OresAdapterInput {\n    value: Value,\n    sha256: String,\n}\n''',
        '''struct OresAdapterInput {\n    value: Value,\n    bytes: Vec<u8>,\n    sha256: String,\n}\n''',
    ),
    (
        '''    return Ok(Some(OresAdapterInput {\n        value,\n        sha256: format!("{:x}", Sha256::digest(&bytes)),\n    }));\n''',
        '''    return Ok(Some(OresAdapterInput {\n        value,\n        sha256: format!("{:x}", Sha256::digest(&bytes)),\n        bytes,\n    }));\n''',
    ),
    (
        '''async fn validate_optional_ores_receipt(\n    path: Option<&str>,\n    provider: &str,\n    runtime_contract: &str,\n    artifact_sha256: &str,\n    adapter_sha256: Option<&str>,\n) -> Result<()> {\n    let Some(path) = path else {\n        return Ok(());\n    };\n''',
        '''async fn validate_optional_ores_receipt(\n    path: Option<&str>,\n    provider: &str,\n    runtime_contract: &str,\n    artifact_sha256: &str,\n    adapter_sha256: Option<&str>,\n) -> Result<Option<Vec<u8>>> {\n    let Some(path) = path else {\n        return Ok(None);\n    };\n''',
    ),
    (
        '''    return Ok(());\n}\n\nasync fn read_json_response''',
        '''    return Ok(Some(bytes));\n}\n\nasync fn read_json_response''',
    ),
    (
        '''            let ores_adapter =\n                read_optional_ores_adapter(config.WASMX_DESKTOP_ORES_ADAPTER.as_deref()).await?;\n            let adapter_supplied = ores_adapter.is_some();\n            let expected_sha256 = format!("{:x}", Sha256::digest(&bytes));\n            validate_optional_ores_receipt(\n                config.WASMX_DESKTOP_ORES_RECEIPT.as_deref(),\n                "wasm_xprs",\n                "wasm-xprs.lambda-runtime/v1",\n                &expected_sha256,\n                ores_adapter.as_ref().map(|adapter| adapter.sha256.as_str()),\n            )\n            .await?;\n            let mut body = json!({\n''',
        '''            let ores_adapter =\n                read_optional_ores_adapter(config.WASMX_DESKTOP_ORES_ADAPTER.as_deref()).await?;\n            let adapter_supplied = ores_adapter.is_some();\n            let expected_sha256 = format!("{:x}", Sha256::digest(&bytes));\n            let receipt_bytes = validate_optional_ores_receipt(\n                config.WASMX_DESKTOP_ORES_RECEIPT.as_deref(),\n                "wasm_xprs",\n                "wasm-xprs.lambda-runtime/v1",\n                &expected_sha256,\n                ores_adapter.as_ref().map(|adapter| adapter.sha256.as_str()),\n            )\n            .await?;\n            let receipt_supplied = receipt_bytes.is_some();\n            let mut body = json!({\n''',
    ),
    (
        '''            if let Some(adapter) = ores_adapter.as_ref() {\n                body["ores_adapter"] = adapter.value.clone();\n            }\n            let response = client\n''',
        '''            if let Some(adapter) = ores_adapter.as_ref() {\n                body["ores_adapter"] = adapter.value.clone();\n            }\n            if let Some(receipt_bytes) = receipt_bytes.as_ref() {\n                let adapter = ores_adapter\n                    .as_ref()\n                    .ok_or_else(|| anyhow!("--ores-receipt requires --ores-adapter"))?;\n                body["ores_adapter_raw_base64"] = Value::String(BASE64.encode(&adapter.bytes));\n                body["ores_receipt_raw_base64"] = Value::String(BASE64.encode(receipt_bytes));\n            }\n            let response = client\n''',
    ),
    (
        '''            validate_deploy_ack(\n                &value,\n                &tenant_id,\n                &deployment_id,\n                &expected_sha256,\n                bytes.len(),\n                adapter_supplied,\n            )?;\n            if config.WASMX_DESKTOP_ORES_RECEIPT.is_some() {\n                let ack = json!({\n                    "schema_version": "ores.lambda.runtime-deploy-ack/v1",\n                    "provider": "wasm_xprs",\n                    "tenant_id": tenant_id,\n                    "deployment_id": deployment_id,\n                    "artifact_sha256": expected_sha256,\n                    "artifact_bytes": bytes.len(),\n                    "adapter_verified": adapter_supplied,\n                    "runtime_contract": "wasm-xprs.lambda-runtime/v1",\n                    "compiled": true\n                });\n                println!("{}", serde_json::to_string_pretty(&ack)?);\n            } else {\n                println!("{}", serde_json::to_string_pretty(&value)?);\n            }\n''',
        '''            validate_deploy_ack(\n                &value,\n                &tenant_id,\n                &deployment_id,\n                &expected_sha256,\n                bytes.len(),\n                adapter_supplied,\n            )?;\n            validate_receipt_ack(&value, receipt_supplied)?;\n            println!("{}", serde_json::to_string_pretty(&value)?);\n''',
    ),
    (
        '''fn validate_status_contract(value: &Value) -> Result<()> {\n''',
        '''fn validate_receipt_ack(value: &Value, receipt_supplied: bool) -> Result<()> {\n    if value.get("ores_receipt_verified").and_then(Value::as_bool) != Some(receipt_supplied) {\n        bail!(\n            "daemon deploy response receipt verification did not match whether ORES receipt evidence was supplied"\n        );\n    }\n    return Ok(());\n}\n\nfn validate_status_contract(value: &Value) -> Result<()> {\n''',
    ),
    (
        '''    #[test]\n    fn ores_mode_ack_has_normalized_contract_shape() -> Result<()> {\n        let ack = json!({\n            "schema_version": "ores.lambda.runtime-deploy-ack/v1",\n            "provider": "wasm_xprs",\n            "tenant_id": "tenant-a",\n            "deployment_id": "release-1",\n            "artifact_sha256": "a".repeat(64),\n            "artifact_bytes": 42,\n            "adapter_verified": true,\n            "runtime_contract": "wasm-xprs.lambda-runtime/v1",\n            "compiled": true\n        });\n        assert_eq!(ack["schema_version"], "ores.lambda.runtime-deploy-ack/v1");\n        assert_eq!(ack["compiled"], true);\n        return Ok(());\n    }\n''',
        '''    #[test]\n    fn receipt_ack_requires_exact_daemon_verification_state() -> Result<()> {\n        validate_receipt_ack(&json!({"ores_receipt_verified": true}), true)?;\n        validate_receipt_ack(&json!({"ores_receipt_verified": false}), false)?;\n        assert!(validate_receipt_ack(&json!({"ores_receipt_verified": false}), true).is_err());\n        assert!(validate_receipt_ack(&json!({}), false).is_err());\n        return Ok(());\n    }\n''',
    ),
]

for old, new in replacements:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f'expected exactly one match, found {count}: {old[:80]!r}')
    text = text.replace(old, new, 1)

path.write_text(text)
