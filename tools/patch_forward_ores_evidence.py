from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()


def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected exactly one match, found {count}: {old[:100]!r}")
    text = text.replace(old, new, 1)


replace_once(
    '''            validate_optional_ores_receipt(\n                config.WASMX_DESKTOP_ORES_RECEIPT.as_deref(),\n                "wasm_xprs",\n                "wasm-xprs.lambda-runtime/v1",\n                &expected_sha256,\n                ores_adapter.as_ref().map(|adapter| adapter.sha256.as_str()),\n            )\n            .await?;''',
    '''            let ores_receipt = validate_optional_ores_receipt(\n                config.WASMX_DESKTOP_ORES_RECEIPT.as_deref(),\n                "wasm_xprs",\n                "wasm-xprs.lambda-runtime/v1",\n                &expected_sha256,\n                ores_adapter.as_ref().map(|adapter| adapter.sha256.as_str()),\n            )\n            .await?;\n            let receipt_supplied = ores_receipt.is_some();''',
)

replace_once(
    '''            if let Some(adapter) = ores_adapter.as_ref() {\n                body["ores_adapter"] = adapter.value.clone();\n            }''',
    '''            if let Some(adapter) = ores_adapter.as_ref() {\n                body["ores_adapter"] = adapter.value.clone();\n            }\n            if let Some(receipt_bytes) = ores_receipt.as_ref() {\n                let adapter = ores_adapter\n                    .as_ref()\n                    .expect("validated ORES receipt requires an ORES adapter");\n                body["ores_adapter_raw_base64"] = Value::String(BASE64.encode(&adapter.bytes));\n                body["ores_receipt_raw_base64"] = Value::String(BASE64.encode(receipt_bytes));\n            }''',
)

replace_once(
    '''            validate_deploy_ack(\n                &value,\n                &tenant_id,\n                &deployment_id,\n                &expected_sha256,\n                bytes.len(),\n                adapter_supplied,\n            )?;''',
    '''            validate_deploy_ack(\n                &value,\n                &tenant_id,\n                &deployment_id,\n                &expected_sha256,\n                bytes.len(),\n                adapter_supplied,\n            )?;\n            validate_receipt_ack(&value, receipt_supplied)?;''',
)

replace_once(
    '''struct OresAdapterInput {\n    value: Value,\n    sha256: String,\n}''',
    '''struct OresAdapterInput {\n    value: Value,\n    sha256: String,\n    bytes: Vec<u8>,\n}''',
)

replace_once(
    '''    return Ok(Some(OresAdapterInput {\n        value,\n        sha256: format!("{:x}", Sha256::digest(&bytes)),\n    }));''',
    '''    return Ok(Some(OresAdapterInput {\n        value,\n        sha256: format!("{:x}", Sha256::digest(&bytes)),\n        bytes,\n    }));''',
)

replace_once(
    ''') -> Result<()> {\n    let Some(path) = path else {\n        return Ok(());\n    };''',
    ''') -> Result<Option<Vec<u8>>> {\n    let Some(path) = path else {\n        return Ok(None);\n    };''',
)

replace_once(
    '''    return Ok(());\n}\n\nasync fn read_json_response''',
    '''    return Ok(Some(bytes));\n}\n\nasync fn read_json_response''',
)

replace_once(
    '''fn validate_status_contract(value: &Value) -> Result<()> {''',
    '''fn validate_receipt_ack(value: &Value, receipt_supplied: bool) -> Result<()> {\n    if value.get("ores_receipt_verified").and_then(Value::as_bool) != Some(receipt_supplied) {\n        bail!(\n            "daemon deploy response receipt verification did not match whether ORES receipt evidence was supplied"\n        );\n    }\n    return Ok(());\n}\n\nfn validate_status_contract(value: &Value) -> Result<()> {''',
)

replace_once(
    '''            "compiled": true,\n            "ores_adapter_verified": true\n        });''',
    '''            "compiled": true,\n            "ores_adapter_verified": true,\n            "ores_receipt_verified": true\n        });''',
)

replace_once(
    '''        validate_deploy_ack(&good, "tenant-a", "release-1", expected_sha256, 42, true)?;''',
    '''        validate_deploy_ack(&good, "tenant-a", "release-1", expected_sha256, 42, true)?;\n        validate_receipt_ack(&good, true)?;''',
)

replace_once(
    '''            "compiled": true,\n            "ores_adapter_verified": false\n        });\n        return validate_deploy_ack(&value, "tenant-a", "release-2", expected_sha256, 7, false);''',
    '''            "compiled": true,\n            "ores_adapter_verified": false,\n            "ores_receipt_verified": false\n        });\n        validate_deploy_ack(&value, "tenant-a", "release-2", expected_sha256, 7, false)?;\n        return validate_receipt_ack(&value, false);''',
)

replace_once(
    '''        validate_optional_ores_receipt(\n            path.to_str(),\n            "wasm_xprs",\n            "wasm-xprs.lambda-runtime/v1",\n            &"a".repeat(64),\n            Some(&"b".repeat(64)),\n        )\n        .await?;''',
    '''        let validated = validate_optional_ores_receipt(\n            path.to_str(),\n            "wasm_xprs",\n            "wasm-xprs.lambda-runtime/v1",\n            &"a".repeat(64),\n            Some(&"b".repeat(64)),\n        )\n        .await?;\n        assert!(validated.is_some());''',
)

path.write_text(text)
