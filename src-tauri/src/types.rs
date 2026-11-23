use serde::{Deserialize, Serialize};
use std::sync::Mutex;

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
pub struct CodingPlanInfo {
    pub level: String,
    /// 套餐版本：V2 按量（TOKENS_LIMIT），V3 积分制（CREDIT_LIMIT）；未知或旧缓存为空
    #[serde(default)]
    pub plan_version: String,
    pub hour5_percentage: i64,
    pub hour5_next_reset: i64,
    pub weekly_percentage: i64,
    pub weekly_next_reset: i64,
    pub mcp_total: i64,
    pub mcp_used: i64,
    pub mcp_remaining: i64,
    pub mcp_next_reset: i64,
    /// V3（积分制）5 小时窗口的积分总量 / 已用 / 剩余，V2（TOKENS_LIMIT）为 0。
    /// 前端会把本地缓存回传，旧缓存没有这些字段，缺失时取默认值
    #[serde(default)]
    pub hour5_credits_total: i64,
    #[serde(default)]
    pub hour5_credits_used: i64,
    #[serde(default)]
    pub hour5_credits_remaining: i64,
    /// V3（积分制）每周窗口的积分总量 / 已用 / 剩余，V2 为 0
    #[serde(default)]
    pub weekly_credits_total: i64,
    #[serde(default)]
    pub weekly_credits_used: i64,
    #[serde(default)]
    pub weekly_credits_remaining: i64,
}

/// quota/limit 接口中的单个额度窗口
#[derive(Default)]
struct QuotaLimit {
    percentage: i64,
    next_reset: i64,
    total: i64,
    used: i64,
    remaining: i64,
}

impl CodingPlanInfo {
    /// 解析 quota/limit 接口的 data 字段。
    ///
    /// 套餐窗口（V2 的 TOKENS_LIMIT、V3 积分制的 CREDIT_LIMIT）按出现顺序依次为
    /// 5 小时、每周；TIME_LIMIT 为 MCP 月度调用次数。
    pub fn from_api(data: &serde_json::Value) -> Self {
        // with_counts 为 false 时不读取 usage/currentValue/remaining
        fn limit(v: &serde_json::Value, with_counts: bool) -> QuotaLimit {
            let num = |key: &str| v.get(key).and_then(|x| x.as_i64()).unwrap_or(0);
            QuotaLimit {
                percentage: num("percentage"),
                next_reset: num("nextResetTime"),
                total: if with_counts { num("usage") } else { 0 },
                used: if with_counts { num("currentValue") } else { 0 },
                remaining: if with_counts { num("remaining") } else { 0 },
            }
        }

        let mut windows: Vec<QuotaLimit> = Vec::new();
        let mut mcp = QuotaLimit::default();
        let mut has_credit_limit = false;
        let mut has_v2_limit = false;

        if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
            for item in limits {
                let limit_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match limit_type {
                    "TIME_LIMIT" => {
                        has_v2_limit = true;
                        mcp = limit(item, true);
                    }
                    // 只有 CREDIT_LIMIT 的 usage/currentValue/remaining 是积分用量
                    "TOKENS_LIMIT" | "CREDIT_LIMIT" => {
                        if limit_type == "CREDIT_LIMIT" {
                            has_credit_limit = true;
                        } else {
                            has_v2_limit = true;
                        }
                        windows.push(limit(item, limit_type == "CREDIT_LIMIT"))
                    }
                    _ => {}
                }
            }
        }

        let mut windows = windows.into_iter();
        let hour5 = windows.next().unwrap_or_default();
        let weekly = windows.next().unwrap_or_default();

        Self {
            level: data
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            plan_version: if has_credit_limit {
                "v3"
            } else if has_v2_limit {
                "v2"
            } else {
                ""
            }
            .to_string(),
            hour5_percentage: hour5.percentage,
            hour5_next_reset: hour5.next_reset,
            weekly_percentage: weekly.percentage,
            weekly_next_reset: weekly.next_reset,
            mcp_total: mcp.total,
            mcp_used: mcp.used,
            mcp_remaining: mcp.remaining,
            mcp_next_reset: mcp.next_reset,
            hour5_credits_total: hour5.total,
            hour5_credits_used: hour5.used,
            hour5_credits_remaining: hour5.remaining,
            weekly_credits_total: weekly.total,
            weekly_credits_used: weekly.used,
            weekly_credits_remaining: weekly.remaining,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ClaudeCodeStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub config_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ClaudeCodeConfig {
    pub model: Option<String>,
    pub anthropic_auth_token: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub anthropic_default_haiku_model: Option<String>,
    pub anthropic_default_sonnet_model: Option<String>,
    pub anthropic_default_opus_model: Option<String>,
    pub api_timeout_ms: Option<String>,
    pub broken_plugins: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ZCodeStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub config_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ZCodeProvider {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub source: String,
    pub api_key: String,
    pub base_url: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ZCodeConfig {
    pub config_path: String,
    pub providers: Vec<ZCodeProvider>,
}

#[derive(Deserialize, Clone)]
pub struct ZCodeProviderInput {
    pub id: String,
    pub api_key: String,
    pub base_url: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct UsageBucket {
    /// 不含缓存的净输入（Zcode 的 input_tokens 已在采集端拆出缓存部分）
    pub input: i64,
    /// 已包含 reasoning（两个数据源的 reasoning 均为 output 子集）
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// 仅展示用，不计入 total（已含在 output 内）
    pub reasoning: i64,
    pub requests: i64,
}

impl UsageBucket {
    pub fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct DayStats {
    pub date: String,
    pub zcode: UsageBucket,
    pub claude: UsageBucket,
    /// 当日按模型明细，供前端按模型过滤
    pub models: Vec<ModelStats>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ModelStats {
    pub model: String,
    pub source: String,
    /// 供应商显示名（仅 Zcode 有；如 "Bigmodel - API Key"），空表示未知
    pub provider: String,
    pub usage: UsageBucket,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TokenStatsResult {
    pub zcode_detected: bool,
    pub claude_detected: bool,
    /// 聚合粒度："day" 或 "hour"（今天视图按小时）
    pub granularity: String,
    pub zcode_sessions: i64,
    pub claude_sessions: i64,
    pub totals_zcode: UsageBucket,
    pub totals_claude: UsageBucket,
    pub by_day: Vec<DayStats>,
    pub by_model: Vec<ModelStats>,
}

#[derive(Default)]
pub struct TrayData {
    pub balance: Option<BalanceInfo>,
    pub coding_plan: Option<CodingPlanInfo>,
}

pub struct AppState {
    pub client: reqwest::Client,
    pub refresh_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub tray_data: Mutex<TrayData>,
    pub minimize_to_tray: Mutex<bool>,
    /// 主窗口隐藏时刻，用于判断是否需要重新加载 WebView
    pub main_hidden_at: Mutex<Option<std::time::Instant>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-23 用 V3（积分制）Pro 账号实测的 quota/limit data
    const V3_CREDIT_PLAN: &str = r#"{
        "limits": [
            {"type": "CREDIT_LIMIT", "unit": 3, "number": 5, "usage": 12000, "currentValue": 59, "remaining": 11940, "percentage": 1, "nextResetTime": 1795459844074},
            {"type": "CREDIT_LIMIT", "unit": 6, "number": 1, "usage": 60000, "currentValue": 5134, "remaining": 54865, "percentage": 8, "nextResetTime": 1796020004983}
        ],
        "level": "pro"
    }"#;

    /// V2 套餐：按量百分比 + MCP 月度调用
    const V2_TOKENS_PLAN: &str = r#"{
        "limits": [
            {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "usage": 12000, "currentValue": 59, "percentage": 18, "nextResetTime": 1795455522827},
            {"type": "TOKENS_LIMIT", "unit": 6, "number": 1, "percentage": 36, "nextResetTime": 1795745070983},
            {"type": "TIME_LIMIT", "unit": 5, "number": 1, "usage": 4000, "currentValue": 365, "remaining": 3635, "percentage": 9, "nextResetTime": 1795917870984}
        ],
        "level": "max"
    }"#;

    #[test]
    fn parses_v3_credit_plan() {
        let plan = CodingPlanInfo::from_api(&serde_json::from_str(V3_CREDIT_PLAN).unwrap());

        assert_eq!(plan.level, "pro");
        assert_eq!(plan.plan_version, "v3");
        assert_eq!(plan.hour5_percentage, 1);
        assert_eq!(plan.hour5_next_reset, 1795459844074);
        assert_eq!(
            (plan.hour5_credits_total, plan.hour5_credits_used, plan.hour5_credits_remaining),
            (12000, 59, 11940)
        );
        assert_eq!(plan.weekly_percentage, 8);
        assert_eq!(plan.weekly_next_reset, 1796020004983);
        assert_eq!(
            (plan.weekly_credits_total, plan.weekly_credits_used, plan.weekly_credits_remaining),
            (60000, 5134, 54865)
        );
        // V3 实测不返回 MCP 月度额度
        assert_eq!((plan.mcp_total, plan.mcp_used, plan.mcp_next_reset), (0, 0, 0));
    }

    #[test]
    fn parses_v2_tokens_plan() {
        let plan = CodingPlanInfo::from_api(&serde_json::from_str(V2_TOKENS_PLAN).unwrap());

        assert_eq!(plan.level, "max");
        assert_eq!(plan.plan_version, "v2");
        assert_eq!(plan.hour5_percentage, 18);
        assert_eq!(plan.hour5_next_reset, 1795455522827);
        assert_eq!(plan.weekly_percentage, 36);
        assert_eq!(plan.weekly_next_reset, 1795745070983);
        assert_eq!((plan.mcp_total, plan.mcp_used, plan.mcp_remaining), (4000, 365, 3635));
        assert_eq!(plan.mcp_next_reset, 1795917870984);
        // TOKENS_LIMIT 即使带 usage 等字段也不是积分用量
        assert_eq!(
            (plan.hour5_credits_total, plan.hour5_credits_used, plan.hour5_credits_remaining),
            (0, 0, 0)
        );
        assert_eq!(plan.weekly_credits_total, 0);
    }

    #[test]
    fn parses_plan_without_limits() {
        let plan = CodingPlanInfo::from_api(&serde_json::json!({ "level": "lite" }));

        assert_eq!(plan.level, "lite");
        assert_eq!(plan.plan_version, "");
        assert_eq!(
            (plan.hour5_percentage, plan.hour5_next_reset, plan.hour5_credits_total),
            (0, 0, 0)
        );
    }

    #[test]
    fn deserializes_plan_cached_before_credits() {
        // 加积分字段之前写入的本地缓存会被前端原样回传
        let plan: CodingPlanInfo = serde_json::from_str(
            r#"{"level":"pro","hour5_percentage":1,"hour5_next_reset":1795459844074,
                "weekly_percentage":8,"weekly_next_reset":1796020004983,
                "mcp_total":0,"mcp_used":0,"mcp_remaining":0,"mcp_next_reset":0}"#,
        )
        .unwrap();

        assert_eq!(plan.level, "pro");
        assert_eq!(plan.plan_version, "");
        assert_eq!(plan.hour5_percentage, 1);
        assert_eq!(plan.hour5_credits_total, 0);
        assert_eq!(plan.weekly_credits_used, 0);
    }
}
