use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Serialize, Deserialize, Clone)]
pub struct BalanceInfo {
    pub balance: f64,
    pub recharge_amount: f64,
    pub give_amount: f64,
    pub total_spend_amount: f64,
    pub frozen_balance: f64,
    pub available_balance: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenCountResult {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CodingQuotaLimit {
    pub limit_type: String,
    pub percentage: i64,
    pub usage: i64,
    pub current_value: i64,
    pub remaining: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CodingPlanInfo {
    pub level: String,
    pub hour5_percentage: i64,
    pub hour5_next_reset: i64,
    pub weekly_percentage: i64,
    pub weekly_next_reset: i64,
    pub mcp_total: i64,
    pub mcp_used: i64,
    pub mcp_remaining: i64,
    pub mcp_next_reset: i64,
}

#[tauri::command]
async fn query_balance(state: tauri::State<'_, AppState>, api_key: String) -> Result<BalanceInfo, String> {
    let client = state.client.clone();
    log::info!("查询余额...");
    let resp = client
        .get("https://www.bigmodel.cn/api/biz/account/query-customer-account-report")
        .header("Authorization", &api_key)
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| {
            log::error!("余额查询请求失败: {}", e);
            format!("请求失败: {}", e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log::error!("余额查询 API 错误 ({}): {}", status, body);
        return Err(format!("API 返回错误 ({}): {}", status, body));
    }

    log::info!("余额查询成功");

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let data = if let Some(d) = json.get("data") {
        d
    } else if json.get("balance").is_some() || json.get("availableBalance").is_some() {
        &json
    } else {
        return Err(format!("未知的响应格式: {}", json));
    };

    Ok(BalanceInfo {
        balance: data
            .get("balance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        recharge_amount: data
            .get("rechargeAmount")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        give_amount: data
            .get("giveAmount")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        total_spend_amount: data
            .get("totalSpendAmount")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        frozen_balance: data
            .get("frozenBalance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        available_balance: data
            .get("availableBalance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    })
}

#[tauri::command]
async fn query_coding_plan(state: tauri::State<'_, AppState>, api_key: String) -> Result<CodingPlanInfo, String> {
    let client = state.client.clone();
    log::info!("查询 Coding Plan...");
    let resp = client
        .get("https://open.bigmodel.cn/api/monitor/usage/quota/limit")
        .header("Authorization", &api_key)
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| {
            log::error!("Coding Plan 查询请求失败: {}", e);
            format!("请求失败: {}", e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log::error!("Coding Plan API 错误 ({}): {}", status, body);
        return Err(format!("API 返回错误 ({}): {}", status, body));
    }

    log::info!("Coding Plan 查询成功");

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let success = json.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    if !success {
        let msg = json
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(format!("查询失败: {}", msg));
    }

    let data = json
        .get("data")
        .ok_or_else(|| format!("响应中无 data 字段: {}", json))?;

    let level = data
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let limits = data
        .get("limits")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // 解析 limits: TIME_LIMIT (MCP月度), TOKENS_LIMIT (5小时 & 每周)
    let mut hour5_percentage: i64 = 0;
    let mut hour5_next_reset: i64 = 0;
    let mut weekly_percentage: i64 = 0;
    let mut weekly_next_reset: i64 = 0;
    let mut mcp_total: i64 = 0;
    let mut mcp_used: i64 = 0;
    let mut mcp_remaining: i64 = 0;
    let mut mcp_next_reset: i64 = 0;
    let mut tokens_limit_count = 0;

    for limit in &limits {
        let limit_type = limit
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let percentage = limit
            .get("percentage")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let next_reset = limit
            .get("nextResetTime")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        match limit_type {
            "TIME_LIMIT" => {
                mcp_total = limit.get("usage").and_then(|v| v.as_i64()).unwrap_or(0);
                mcp_used = limit.get("currentValue").and_then(|v| v.as_i64()).unwrap_or(0);
                mcp_remaining = limit.get("remaining").and_then(|v| v.as_i64()).unwrap_or(0);
                mcp_next_reset = next_reset;
            }
            "TOKENS_LIMIT" => {
                if tokens_limit_count == 0 {
                    hour5_percentage = percentage;
                    hour5_next_reset = next_reset;
                } else {
                    weekly_percentage = percentage;
                    weekly_next_reset = next_reset;
                }
                tokens_limit_count += 1;
            }
            _ => {}
        }
    }

    Ok(CodingPlanInfo {
        level,
        hour5_percentage,
        hour5_next_reset,
        weekly_percentage,
        weekly_next_reset,
        mcp_total,
        mcp_used,
        mcp_remaining,
        mcp_next_reset,
    })
}

#[tauri::command]
async fn count_tokens(state: tauri::State<'_, AppState>, api_key: String, text: String, model: String) -> Result<TokenCountResult, String> {
    let client = state.client.clone();

    log::info!("Token 计算 (model={}): {}", model, if text.len() > 50 { format!("{}...", &text[..50]) } else { text.clone() });
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": text
            }
        ],
        "max_tokens": 1
    });

    let resp = client
        .post("https://open.bigmodel.cn/api/paas/v4/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            log::error!("Token 计算请求失败: {}", e);
            format!("请求失败: {}", e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log::error!("Token 计算 API 错误 ({}): {}", status, body);
        return Err(format!("API 返回错误 ({}): {}", status, body));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let usage = json
        .get("usage")
        .ok_or_else(|| format!("响应中无 usage 字段: {}", json))?;

    let result = TokenCountResult {
        prompt_tokens: usage
            .get("prompt_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        completion_tokens: usage
            .get("completion_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        total_tokens: usage
            .get("total_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    };
    log::info!("Token 计算完成: prompt={}, completion={}, total={}", result.prompt_tokens, result.completion_tokens, result.total_tokens);
    Ok(result)
}

struct AppState {
    client: reqwest::Client,
    refresh_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[tauri::command]
async fn start_auto_refresh(app: tauri::AppHandle, state: tauri::State<'_, AppState>, api_key: String, interval_secs: u64) -> Result<(), String> {
    // 停止旧任务
    if let Some(h) = state.refresh_handle.lock().unwrap().take() {
        h.abort();
    }

    let client = state.client.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

        loop {
            interval.tick().await;
            log::info!("[自动刷新] 开始轮询...");

            let balance_resp = client
                .get("https://www.bigmodel.cn/api/biz/account/query-customer-account-report")
                .header("Authorization", &api_key)
                .header("Content-Type", "application/json")
                .send()
                .await;

            match &balance_resp {
                Ok(resp) if resp.status().is_success() => log::info!("[自动刷新] 余额查询成功"),
                Ok(resp) => log::warn!("[自动刷新] 余额查询返回 {}", resp.status()),
                Err(e) => log::error!("[自动刷新] 余额查询失败: {}", e),
            }

            if let Ok(resp) = balance_resp {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let data = json.get("data").cloned().unwrap_or(json);
                    let _ = app.emit("balance-update", &data);
                }
            }

            let plan_resp = client
                .get("https://open.bigmodel.cn/api/monitor/usage/quota/limit")
                .header("Authorization", &api_key)
                .header("Content-Type", "application/json")
                .send()
                .await;

            match &plan_resp {
                Ok(resp) if resp.status().is_success() => log::info!("[自动刷新] Coding Plan 查询成功"),
                Ok(resp) => log::warn!("[自动刷新] Coding Plan 查询返回 {}", resp.status()),
                Err(e) => log::error!("[自动刷新] Coding Plan 查询失败: {}", e),
            }

            if let Ok(resp) = plan_resp {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if json.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                        if let Some(data) = json.get("data").cloned() {
                            let _ = app.emit("plan-update", &data);
                        }
                    }
                }
            }
        }
    });

    *state.refresh_handle.lock().unwrap() = Some(handle);
    Ok(())
}

#[tauri::command]
async fn stop_auto_refresh(state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Some(h) = state.refresh_handle.lock().unwrap().take() {
        h.abort();
    }
    Ok(())
}

#[tauri::command]
async fn open_devtools(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(debug_assertions)]
        window.open_devtools();
        #[cfg(not(debug_assertions))]
        {
            let _ = window;
        }
    }
}

#[tauri::command]
async fn get_app_info() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "name": "zhipu-token-tool",
        "arch": std::env::consts::ARCH,
        "os": std::env::consts::OS,
        "family": std::env::consts::FAMILY,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .manage(AppState {
            client: reqwest::Client::new(),
            refresh_handle: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            query_balance,
            query_coding_plan,
            count_tokens,
            start_auto_refresh,
            stop_auto_refresh,
            open_devtools,
            get_app_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
