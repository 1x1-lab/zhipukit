// zhipukit-claude-code-plugin: Claude Code statusline 工具
// 读取 ~/.claude/settings.json 中的 ANTHROPIC_AUTH_TOKEN 和 zhipuEndpoint，查询智谱 API，输出套餐信息到 stdout
// 支持 statusline 模式（缓存时间可配置，默认 5 分钟）和独立测试模式

use app_lib::utils::{
    balance_base_url, build_url, format_amount, format_remaining, format_status_bar,
    API_PATH_BALANCE, API_PATH_CODING_PLAN,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use terminal_size::{terminal_size, Width};

/// 默认 endpoint（国内版）
const DEFAULT_ENDPOINT: &str = "https://open.bigmodel.cn";

fn get_home_dir() -> Result<PathBuf, String> {
    if cfg!(windows) {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .map_err(|_| "Cannot determine home directory".to_string())
    } else {
        std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "Cannot determine home directory".to_string())
    }
}

/// 带 ANSI 颜色进度条 + 百分比
fn progress_bar_pct(percentage: i64, length: usize) -> String {
    let bar = format_status_bar(percentage, length);
    format!("{} {}%", bar, percentage)
}

/// 格式化上下文窗口使用进度
fn format_context_usage(ctx: &ClaudeContext) -> String {
    let pct = ctx.buffered_percent();
    let bar = format_status_bar(pct, 8);

    // 基础显示
    let mut result = if ctx.current_tokens > 0 && ctx.context_window_size > 0 {
        let size_k = ctx.context_window_size / 1000;
        format!(
            "{} {}% ({:.1}k/{}k)",
            bar,
            pct,
            ctx.current_tokens as f64 / 1000.0,
            size_k
        )
    } else {
        format!("{} {}%", bar, pct)
    };

    // 高用量 (≥85%) 时追加 input/cache token 明细
    if pct >= 85 && (ctx.input_tokens > 0 || ctx.cache_tokens > 0) {
        let in_k = ctx.input_tokens as f64 / 1000.0;
        let cache_k = ctx.cache_tokens as f64 / 1000.0;
        result.push_str(&format!(" (in: {:.1}k, cache: {:.1}k)", in_k, cache_k));
    }

    result
}

/// Autocompact 缓冲比例（经验值，匹配 Claude Code /context 输出）
const AUTOCOMPACT_BUFFER_PERCENT: f64 = 0.165;
/// 缓冲缩放下限：≤5% 使用率时无缓冲
const BUFFER_SCALE_LOW: f64 = 0.05;
/// 缓冲缩放上限：≥50% 使用率时满缓冲
const BUFFER_SCALE_HIGH: f64 = 0.50;

/// 从 Claude Code stdin JSON 解析的上下文窗口信息
struct ClaudeContext {
    used_percentage: Option<i64>,
    context_window_size: i64,
    /// 当前上下文实际占用（input + cache_creation + cache_read）
    current_tokens: i64,
    /// input_tokens 分项
    input_tokens: i64,
    /// cache_creation + cache_read 合计
    cache_tokens: i64,
}

impl ClaudeContext {
    /// 原始百分比：优先用原生值，否则手动计算
    #[allow(dead_code)]
    fn raw_percent(&self) -> i64 {
        if let Some(pct) = self.used_percentage {
            return pct.clamp(0, 100);
        }
        if self.context_window_size <= 0 {
            return 0;
        }
        ((self.current_tokens as f64 / self.context_window_size as f64) * 100.0).round() as i64
    }

    /// 带缓冲的百分比：优先用原生值，否则手动计算 + autocompact 缓冲
    fn buffered_percent(&self) -> i64 {
        if let Some(pct) = self.used_percentage {
            return pct.clamp(0, 100);
        }
        if self.context_window_size <= 0 {
            return 0;
        }
        let raw_ratio = self.current_tokens as f64 / self.context_window_size as f64;
        // 缓冲缩放：低使用率无缓冲，高使用率满缓冲
        let scale = ((raw_ratio - BUFFER_SCALE_LOW) / (BUFFER_SCALE_HIGH - BUFFER_SCALE_LOW))
            .clamp(0.0, 1.0);
        let buffer = self.context_window_size as f64 * AUTOCOMPACT_BUFFER_PERCENT * scale;
        (((self.current_tokens as f64 + buffer) / self.context_window_size as f64 * 100.0)
            .round() as i64)
            .clamp(0, 100)
    }
}

/// stdin 解析结果：上下文窗口信息 + 当前工作目录 + 当前模型
struct StdinData {
    context: ClaudeContext,
    cwd: Option<String>,
    model: Option<String>,
}

/// 解析 Claude Code 通过 stdin 传入的 JSON，提取上下文窗口信息和项目路径
/// 返回 None 如果没有 stdin 数据或解析失败（向后兼容）
fn parse_stdin_data() -> Option<StdinData> {
    if io::stdin().is_terminal() {
        return None;
    }
    let mut buf = String::new();
    if io::stdin().lock().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(&buf).ok()?;
    let cw = json.get("context_window")?;

    // 从 current_usage 获取实际上下文占用
    let (input_tokens, cache_tokens, current_tokens) = cw
        .get("current_usage")
        .map(|u| {
            let input = u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let cache_create = u
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache_read = u
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache = cache_create + cache_read;
            (input, cache, input + cache)
        })
        .unwrap_or((0, 0, 0));

    let context = ClaudeContext {
        used_percentage: cw.get("used_percentage").and_then(|v| v.as_i64()),
        context_window_size: cw
            .get("context_window_size")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        current_tokens,
        input_tokens,
        cache_tokens,
    };

    let cwd = json
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let model = json
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    Some(StdinData { context, cwd, model })
}

/// 获取指定目录的 git 分支名
fn get_git_branch(dir: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(dir)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let branch = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if branch.is_empty() { None } else { Some(branch) }
            } else {
                None
            }
        })
}

/// 对 api_key + endpoint 计算 hash 摘要（前 16 位 hex）
fn config_hash(api_key: &str, endpoint: &str) -> String {
    let mut hasher = DefaultHasher::new();
    format!("{}\n{}", api_key, endpoint).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 提前读取 settings.json 中的 api_key、endpoint 和 model（用于缓存校验和模型展示）
fn read_config_keys() -> (Option<String>, Option<String>, Option<String>) {
    let home = match get_home_dir() {
        Ok(h) => h,
        Err(_) => return (None, None, None),
    };
    let config_path = home.join(".claude").join("settings.json");
    if !config_path.exists() {
        return (None, None, None);
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return (None, None, None),
    };
    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };

    let api_key = config
        .get("env")
        .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let endpoint = Some(
        config
            .get("zhipuEndpoint")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_ENDPOINT)
            .to_string(),
    );

    let model = config
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    (api_key, endpoint, model)
}

/// 缓存文件路径
fn cache_path() -> Result<PathBuf, String> {
    Ok(get_home_dir()?.join(".claude").join("zhipukit-cache.json"))
}

/// 从 settings.json 读取缓存有效期（秒），默认 300 秒（5 分钟）
fn read_cache_duration() -> i64 {
    let home = match get_home_dir() {
        Ok(h) => h,
        Err(_) => return 300,
    };
    let config_path = home.join(".claude").join("settings.json");
    if !config_path.exists() {
        return 300;
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return 300,
    };
    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return 300,
    };
    config
        .get("zhipuCacheDuration")
        .and_then(|v| v.as_i64())
        .filter(|&d| d > 0)
        .unwrap_or(300)
}

/// 尝试读取缓存（有效期内且 key_hash 匹配），返回缓存的输出文本
fn read_cache(api_key: &str, endpoint: &str) -> Option<String> {
    let path = cache_path().ok()?;
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let cached_at = json.get("cached_at").and_then(|v| v.as_i64())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;

    // 缓存有效期（从 settings.json 读取，默认 5 分钟）
    let cache_duration_ms = read_cache_duration() * 1000;
    if now - cached_at > cache_duration_ms {
        return None;
    }

    // 校验 key_hash，确保缓存来源与当前配置一致
    let expected_hash = config_hash(api_key, endpoint);
    let cached_hash = json.get("key_hash").and_then(|v| v.as_str()).unwrap_or("");
    if cached_hash != expected_hash {
        return None;
    }

    json.get("output")
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// 写入缓存
fn write_cache(output: &str, api_key: &str, endpoint: &str) {
    if let Ok(path) = cache_path() {
        let json = serde_json::json!({
            "cached_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            "key_hash": config_hash(api_key, endpoint),
            "output": output
        });
        let _ = std::fs::write(&path, json.to_string());
    }
}

/// 获取终端宽度，默认 80
fn term_width() -> usize {
    terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80)
}

/// 将模型名添加到输出第一行前面
fn prepend_model(output: &str, model: Option<&str>) -> String {
    let Some(m) = model else {
        return output.to_string();
    };
    let short = short_model_name(m);
    let lines: Vec<&str> = output.lines().collect();
    if let Some(first) = lines.first() {
        let rest = if lines.len() > 1 {
            format!("\n{}", lines[1..].join("\n"))
        } else {
            String::new()
        };
        format!("[{}] {}{}", short, first, rest)
    } else {
        format!("[{}]", short)
    }
}

/// 将所有行按 " | " 分割成段，根据终端宽度自动换行重排
fn wrap_lines(lines: &[&str]) -> String {
    let width = term_width();
    let mut all_segments: Vec<String> = Vec::new();
    for line in lines {
        for seg in line.split(" | ") {
            if !seg.is_empty() {
                all_segments.push(seg.to_string());
            }
        }
    }

    let mut output_lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for seg in &all_segments {
        if current.is_empty() {
            current = seg.clone();
        } else if current.len() + 3 + seg.len() <= width {
            current = format!("{} | {}", current, seg);
        } else {
            output_lines.push(current);
            current = seg.clone();
        }
    }
    if !current.is_empty() {
        output_lines.push(current);
    }

    output_lines.join("\n")
}

/// 将 git 分支信息追加到 line1
fn append_git_branch(line1: &str, cwd: Option<&str>) -> String {
    if let Some(dir) = cwd {
        if let Some(branch) = get_git_branch(dir) {
            return format!("{} | Git ({})", line1, branch);
        }
    }
    line1.to_string()
}

#[tokio::main]
async fn main() {
    // 提前读取配置，获取 api_key、endpoint 和 model（用于缓存校验和模型展示）
    let (api_key, endpoint, settings_model) = read_config_keys();

    // 解析 stdin 数据（上下文 + 项目路径 + 模型，实时数据不缓存）
    let stdin_data = parse_stdin_data();
    let cwd = stdin_data.as_ref().and_then(|d| d.cwd.clone());
    let stdin_model = stdin_data.as_ref().and_then(|d| d.model.clone());

    // 有效模型：优先用 stdin（Claude Code 实际使用的模型），否则用 settings.json 配置
    let effective_model = stdin_model.as_deref().or(settings_model.as_deref());

    // statusline 模式：优先使用缓存获取 API 部分
    if !io::stdin().is_terminal() {
        if let (Some(ref ak), Some(ref ep)) = (&api_key, &endpoint) {
            if let Some(cached) = read_cache(ak, ep) {
                if let Some(ref data) = stdin_data {
                    let (line1, combined) = merge_context(&data.context, effective_model, &cached);
                    let line1 = append_git_branch(&line1, cwd.as_deref());
                    println!("{}", wrap_lines(&[&line1, &combined]));
                } else {
                    let output = prepend_model(&cached, effective_model);
                    println!("{}", wrap_lines(&[&output]));
                }
                return;
            }
        }
    }

    let result = fetch_and_format().await;
    match result {
        Ok(output) => {
            if let (Some(ref ak), Some(ref ep)) = (&api_key, &endpoint) {
                write_cache(&output, ak, ep);
            }
            if let Some(ref data) = stdin_data {
                let (line1, combined) = merge_context(&data.context, effective_model, &output);
                let line1 = append_git_branch(&line1, cwd.as_deref());
                println!("{}", wrap_lines(&[&line1, &combined]));
            } else {
                let output = prepend_model(&output, effective_model);
                println!("{}", wrap_lines(&[&output]));
            }
        }
        Err(e) => {
            // 出错时也尝试用过期缓存
            if let (Some(ref ak), Some(ref ep)) = (&api_key, &endpoint) {
                if let Some(cached) = read_cache(ak, ep) {
                    if let Some(ref data) = stdin_data {
                        let (line1, combined) = merge_context(&data.context, effective_model, &cached);
                        let line1 = append_git_branch(&line1, cwd.as_deref());
                        println!("{}", wrap_lines(&[&line1, &combined]));
                    } else {
                        let output = prepend_model(&cached, effective_model);
                        println!("{}", wrap_lines(&[&output]));
                    }
                    return;
                }
            }
            if let Some(ref data) = stdin_data {
                let mut parts: Vec<String> = Vec::new();
                if let Some(ref m) = effective_model {
                    parts.push(format!("[{}]", short_model_name(m)));
                }
                parts.push(format_context_usage(&data.context));
                println!("{}", parts.join(" "));
            } else if let Some(ref m) = effective_model {
                eprintln!("[{}] {}", short_model_name(m), e);
            } else {
                eprintln!("[ZhipuKit] {}", e);
            }
        }
    }
}

/// 简化模型名：去掉日期后缀，缩短常见前缀
fn short_model_name(model: &str) -> String {
    // claude-sonnet-4-20250514 -> Sonnet 4
    // claude-opus-4-6-20250514 -> Opus 4.6
    // claude-haiku-4-5-20251001 -> Haiku 4.5
    let s = model.to_lowercase();
    let name = if s.starts_with("claude-") {
        &s[7..]
    } else {
        return model.to_string();
    };
    // 去掉日期后缀 (-20250514)
    let without_date = name.split('-').next_back().map(|last| {
        if last.len() == 8 && last.parse::<u32>().is_ok() {
            // 最后一段是日期，去掉
            name.trim_end_matches(&format!("-{}", last))
        } else {
            name
        }
    }).unwrap_or(name);

    // 提取 tier 和版本
    let parts: Vec<&str> = without_date.split('-').collect();
    let tier = parts.first().unwrap_or(&"");
    let version = if parts.len() >= 2 { parts[1..].join(".") } else { String::new() };

    // 首字母大写
    let tier_cap = match *tier {
        "sonnet" => "Sonnet",
        "opus" => "Opus",
        "haiku" => "Haiku",
        other => other,
    };

    if version.is_empty() {
        tier_cap.to_string()
    } else {
        format!("{} {}", tier_cap, version)
    }
}

/// 合并上下文进度与 API 输出
/// 返回 (第一行, 第二行合并后的 quota 行)
fn merge_context(ctx: &ClaudeContext, model: Option<&str>, api_output: &str) -> (String, String) {
    let ctx_str = format_context_usage(ctx);
    let lines: Vec<&str> = api_output.lines().collect();
    let line1 = lines.first().unwrap_or(&"").to_string();

    // 构建前缀：[模型名] + 上下文进度
    let mut prefix_parts: Vec<String> = Vec::new();
    if let Some(m) = model {
        prefix_parts.push(format!("[{}]", m));
    }
    prefix_parts.push(ctx_str);
    let prefix = prefix_parts.join(" ");

    let combined = if lines.len() >= 2 {
        format!("{} | {}", prefix, lines[1])
    } else {
        prefix
    };
    (line1, combined)
}

struct QuotaData {
    balance: Option<f64>,
    level: Option<String>,
    hour5_pct: Option<i64>,
    hour5_next_reset: Option<i64>,
    weekly_pct: Option<i64>,
    weekly_next_reset: Option<i64>,
    mcp_used: Option<i64>,
    mcp_total: Option<i64>,
    mcp_next_reset: Option<i64>,
}

async fn fetch_and_format() -> Result<String, String> {
    let home = get_home_dir()?;
    let config_path = home.join(".claude").join("settings.json");

    if !config_path.exists() {
        return Err("未找到 Claude Code 配置文件".to_string());
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("读取配置失败: {}", e))?;
    let config: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析配置失败: {}", e))?;

    let api_key = config
        .get("env")
        .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if api_key.is_empty() {
        return Err("ANTHROPIC_AUTH_TOKEN 未配置".to_string());
    }

    // 从 settings.json 读取 zhipuEndpoint，默认国内版
    let endpoint = config
        .get("zhipuEndpoint")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_ENDPOINT)
        .to_string();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut data = QuotaData {
        balance: None,
        level: None,
        hour5_pct: None,
        hour5_next_reset: None,
        weekly_pct: None,
        weekly_next_reset: None,
        mcp_used: None,
        mcp_total: None,
        mcp_next_reset: None,
    };

    let balance_url = build_url(&balance_base_url(&endpoint), API_PATH_BALANCE);
    let plan_url = build_url(&endpoint, API_PATH_CODING_PLAN);

    // 查询余额
    if let Ok(resp) = client
        .get(&balance_url)
        .header("Authorization", &api_key)
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                let d = json.get("data").cloned().unwrap_or(json);
                data.balance =
                    Some(d.get("availableBalance").and_then(|v| v.as_f64()).unwrap_or(0.0));
            }
        }
    }

    // 查询 Coding Plan
    if let Ok(resp) = client
        .get(&plan_url)
        .header("Authorization", &api_key)
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if json
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    if let Some(plan) = json.get("data") {
                        data.level = plan
                            .get("level")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        if let Some(limits) = plan.get("limits").and_then(|v| v.as_array()) {
                            let mut tokens_count = 0;
                            for limit in limits {
                                match limit
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                {
                                    "TIME_LIMIT" => {
                                        data.mcp_total =
                                            limit.get("usage").and_then(|v| v.as_i64());
                                        data.mcp_used =
                                            limit.get("currentValue").and_then(|v| v.as_i64());
                                        data.mcp_next_reset =
                                            limit.get("nextResetTime").and_then(|v| v.as_i64());
                                    }
                                    "TOKENS_LIMIT" => {
                                        let pct = limit
                                            .get("percentage")
                                            .and_then(|v| v.as_i64())
                                            .unwrap_or(0);
                                        let next_reset =
                                            limit.get("nextResetTime").and_then(|v| v.as_i64());
                                        if tokens_count == 0 {
                                            data.hour5_pct = Some(pct);
                                            data.hour5_next_reset = next_reset;
                                        } else {
                                            data.weekly_pct = Some(pct);
                                            data.weekly_next_reset = next_reset;
                                        }
                                        tokens_count += 1;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    // 格式化输出
    let level_str = data.level.as_deref().unwrap_or("unknown");
    let mut line1 = format!("ZhipuKit {}", level_str.to_uppercase());
    if let Some(balance) = data.balance {
        line1.push_str(&format!(" | ¥{}", format_amount(balance)));
    }

    let mut quota_parts: Vec<String> = Vec::new();

    if let Some(pct) = data.hour5_pct {
        let mut s = format!("5h {}", progress_bar_pct(pct, 8));
        if let Some(reset) = data.hour5_next_reset {
            let remaining_ms = (reset - now).max(0);
            let elapsed_ms = (5 * 3600 * 1000 - remaining_ms).max(0);
            s.push_str(&format!(" ({}/5h)", format_remaining(elapsed_ms)));
        }
        quota_parts.push(s);
    }

    if let (Some(used), Some(total)) = (data.mcp_used, data.mcp_total) {
        if total > 0 {
            let pct = (used * 100 / total).min(100);
            let mut s = format!("MCP {}", format_status_bar(pct, 8));
            let mut time_info = format!("{}/{}", used, total);
            if let Some(reset) = data.mcp_next_reset {
                let remaining_ms = (reset - now).max(0);
                let elapsed_ms = (30 * 24 * 3600 * 1000 - remaining_ms).max(0);
                let d = elapsed_ms / (24 * 3600 * 1000);
                let h = (elapsed_ms % (24 * 3600 * 1000)) / (3600 * 1000);
                time_info.push_str(&format!(" | {}d {}h/30d", d, h));
            }
            s.push_str(&format!(" ({})", time_info));
            quota_parts.push(s);
        }
    }

    if quota_parts.is_empty() {
        Ok(line1)
    } else {
        Ok(format!("{}\n{}", line1, quota_parts.join(" | ")))
    }
}
