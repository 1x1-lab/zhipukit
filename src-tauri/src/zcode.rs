use crate::types::{ZCodeConfig, ZCodeProvider, ZCodeProviderInput, ZCodeStatus};
use crate::utils::get_home_dir;

/// Zcode 配置文件路径：~/.zcode/v2/config.json
fn zcode_config_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".zcode").join("v2").join("config.json")
}

/// Zcode 桌面应用的已知安装位置
fn known_zcode_paths() -> Vec<std::path::PathBuf> {
    if cfg!(windows) {
        match std::env::var("LOCALAPPDATA") {
            Ok(local) => vec![std::path::PathBuf::from(local)
                .join("Programs")
                .join("ZCode")
                .join("ZCode.exe")],
            Err(_) => vec![],
        }
    } else if cfg!(target_os = "macos") {
        vec![std::path::PathBuf::from("/Applications/ZCode.app")]
    } else {
        vec![]
    }
}

#[tauri::command]
pub async fn detect_zcode() -> Result<ZCodeStatus, String> {
    let home = get_home_dir().ok();
    let config_path = home
        .as_ref()
        .map(|h| zcode_config_path(h).to_string_lossy().to_string());

    // 配置文件由 Zcode 首次运行时生成，存在即视为已安装
    let config_exists = config_path
        .as_ref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false);
    let app_path = known_zcode_paths().into_iter().find(|p| p.exists());

    let installed = config_exists || app_path.is_some();
    Ok(ZCodeStatus {
        installed,
        // Zcode 是桌面应用，未提供命令行版本查询
        version: None,
        path: app_path.map(|p| p.to_string_lossy().to_string()),
        config_path,
    })
}

#[tauri::command]
pub async fn read_zcode_config() -> Result<ZCodeConfig, String> {
    let home = get_home_dir()?;
    let config_path = zcode_config_path(&home);

    if !config_path.exists() {
        return Err("Zcode 配置文件不存在，请先安装并运行一次 Zcode".to_string());
    }

    let content = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| format!("读取配置失败: {}", e))?;

    let raw: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 JSON 失败: {}", e))?;

    let mut providers = Vec::new();
    if let Some(map) = raw.get("provider").and_then(|p| p.as_object()) {
        for (id, val) in map {
            let options = val.get("options");
            providers.push(ZCodeProvider {
                id: id.clone(),
                name: val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: val
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                source: val
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                api_key: options
                    .and_then(|o| o.get("apiKey"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                base_url: options
                    .and_then(|o| o.get("baseURL"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }

    Ok(ZCodeConfig {
        config_path: config_path.to_string_lossy().to_string(),
        providers,
    })
}

#[tauri::command]
pub async fn save_zcode_config(providers: Vec<ZCodeProviderInput>) -> Result<(), String> {
    let home = get_home_dir()?;
    let config_path = zcode_config_path(&home);

    if !config_path.exists() {
        return Err("Zcode 配置文件不存在，请先安装并运行一次 Zcode".to_string());
    }

    let content = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| format!("读取配置失败: {}", e))?;

    let mut raw: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 JSON 失败: {}", e))?;

    let provider_map = raw
        .get_mut("provider")
        .and_then(|p| p.as_object_mut())
        .ok_or_else(|| "Zcode 配置文件格式异常：缺少 provider 字段".to_string())?;

    for p in &providers {
        // 只更新已存在的 provider，避免写入脏数据
        let Some(entry) = provider_map.get_mut(&p.id) else {
            continue;
        };
        if entry.get("options").is_none() {
            entry["options"] = serde_json::Value::Object(Default::default());
        }
        entry["options"]["apiKey"] = serde_json::Value::String(p.api_key.clone());
        entry["options"]["baseURL"] = serde_json::Value::String(p.base_url.clone());
    }

    let output =
        serde_json::to_string_pretty(&raw).map_err(|e| format!("序列化 JSON 失败: {}", e))?;

    // 写入前备份原配置
    let backup = config_path.with_extension("json.bak");
    let _ = tokio::fs::copy(&config_path, &backup).await;

    tokio::fs::write(&config_path, output)
        .await
        .map_err(|e| format!("写入配置失败: {}", e))?;
    Ok(())
}
