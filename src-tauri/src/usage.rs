use crate::types::{DayStats, ModelStats, TokenStatsResult, UsageBucket};
use crate::utils::get_home_dir;
use chrono::{DateTime, Local, Utc};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// 从一行日志解析出的单次用量记录（date 为原始 ISO 时间戳，由调用方按粒度聚合）
struct UsageRecord {
    date: String,
    model: String,
    usage: UsageBucket,
}

#[tauri::command]
pub async fn query_token_stats(
    days: Option<u64>,
    hours: Option<u64>,
) -> Result<TokenStatsResult, String> {
    tokio::task::spawn_blocking(move || scan_token_stats(days, hours))
        .await
        .map_err(|e| format!("统计任务执行失败: {}", e))?
}

fn scan_token_stats(days: Option<u64>, hours: Option<u64>) -> Result<TokenStatsResult, String> {
    let home = get_home_dir()?;
    let zcode_dir = home.join(".zcode").join("cli").join("rollout");
    let claude_dir = home.join(".claude").join("projects");

    // hours=N：最近 N 小时（按小时聚合）；days=1：今天（按小时聚合）；days>=2：按天；其余：全部
    let hourly = hours.filter(|h| *h > 0).is_some() || days == Some(1);
    let key_fmt = if hourly { "%Y-%m-%d %H:00" } else { "%Y-%m-%d" };

    // cutoff 与时间档 key 同格式，可直接字符串比较
    let cutoff: Option<String> = if let Some(h) = hours.filter(|h| *h > 0) {
        Some(
            (Local::now() - chrono::Duration::hours(h as i64))
                .format(key_fmt)
                .to_string(),
        )
    } else if let Some(d) = days.filter(|d| *d > 0) {
        Some(
            (Local::now() - chrono::Duration::days(d as i64 - 1))
                .format(key_fmt)
                .to_string(),
        )
    } else {
        None
    };

    let mut result = TokenStatsResult {
        zcode_detected: zcode_dir.exists(),
        claude_detected: claude_dir.exists(),
        granularity: if hourly { "hour" } else { "day" }.to_string(),
        ..Default::default()
    };
    let mut by_day: HashMap<String, DayStats> = HashMap::new();
    let mut by_model: HashMap<(String, String), UsageBucket> = HashMap::new(); // (source, model)

    // Zcode: rollout/model-io-*.jsonl
    let mut zcode_files = Vec::new();
    if zcode_dir.exists() {
        collect_jsonl_files(&zcode_dir, "model-io-", &mut zcode_files);
    }
    for f in &zcode_files {
        if skip_by_mtime(f, &cutoff, hourly) {
            continue;
        }
        result.zcode_sessions += 1;
        process_file(
            f,
            &cutoff,
            hourly,
            parse_zcode_line,
            "zcode",
            &mut by_day,
            &mut by_model,
            &mut result.totals_zcode,
        );
    }

    // Claude Code: projects/**/*.jsonl
    let mut claude_files = Vec::new();
    if claude_dir.exists() {
        collect_jsonl_files(&claude_dir, "", &mut claude_files);
    }
    for f in &claude_files {
        if skip_by_mtime(f, &cutoff, hourly) {
            continue;
        }
        result.claude_sessions += 1;
        process_file(
            f,
            &cutoff,
            hourly,
            parse_claude_line,
            "claude",
            &mut by_day,
            &mut by_model,
            &mut result.totals_claude,
        );
    }

    // 按时间粒度升序输出，并补齐区间内缺失的时间档（小时或日期）
    if hourly {
        // 起点：cutoff 小时档（天粒度 cutoff 补 " 00:00"），无 cutoff 用最早记录
        let start = match &cutoff {
            Some(c) if c.len() > 10 => c.clone(),
            Some(c) => format!("{} 00:00", c),
            None => match by_day.keys().min().cloned() {
                Some(first) => first,
                None => String::new(),
            },
        };
        let end = Local::now().format("%Y-%m-%d %H:00").to_string();
        let mut cursor = start;
        while !cursor.is_empty() && cursor <= end {
            by_day.entry(cursor.clone()).or_default().date = cursor.clone();
            let next = next_hour(&cursor);
            if next == cursor {
                break;
            }
            cursor = next;
        }
    } else if let Some(first) = by_day.keys().min().cloned() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut cursor = first;
        while cursor <= today {
            by_day.entry(cursor.clone()).or_default().date = cursor.clone();
            let next = next_date(&cursor);
            if next == cursor {
                break;
            }
            cursor = next;
        }
    }
    let mut dates: Vec<String> = by_day.keys().cloned().collect();
    dates.sort();
    result.by_day = dates.into_iter().filter_map(|d| by_day.remove(&d)).collect();

    // 按总 token 降序输出模型聚合
    let mut models: Vec<ModelStats> = by_model
        .into_iter()
        .map(|((source, model), usage)| ModelStats { model, source, usage })
        .collect();
    models.sort_by(|a, b| b.usage.total().cmp(&a.usage.total()));
    result.by_model = models;

    Ok(result)
}

/// ISO 时间戳转本地时区时间档 key（按天 "YYYY-MM-DD" 或按小时 "YYYY-MM-DD HH:00"）
fn local_key(ts: &str, hourly: bool) -> Option<String> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| {
            let local = dt.with_timezone(&Local);
            if hourly {
                local.format("%Y-%m-%d %H:00").to_string()
            } else {
                local.format("%Y-%m-%d").to_string()
            }
        })
}

/// YYYY-MM-DD 加一天，解析失败时返回原值
fn next_date(date: &str) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.succ_opt())
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| date.to_string())
}

/// 小时档 key "YYYY-MM-DD HH:00" 加一小时，解析失败时返回原值
fn next_hour(key: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:%M")
        .ok()
        .map(|dt| {
            (dt + chrono::Duration::hours(1))
                .format("%Y-%m-%d %H:00")
                .to_string()
        })
        .unwrap_or_else(|| key.to_string())
}

/// 递归收集指定前缀的 .jsonl 文件（prefix 为空表示不过滤文件名）
fn collect_jsonl_files(dir: &Path, prefix: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, prefix, out);
        } else {
            let name_ok = path
                .file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    prefix.is_empty() || n.starts_with(prefix)
                })
                .unwrap_or(false);
            let ext_ok = path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("jsonl"))
                .unwrap_or(false);
            if name_ok && ext_ok {
                out.push(path);
            }
        }
    }
}

/// 文件最后修改时间早于 cutoff 时间档时跳过（文件内所有记录必然不晚于修改时间）
fn skip_by_mtime(path: &Path, cutoff: &Option<String>, hourly: bool) -> bool {
    let Some(cutoff) = cutoff else {
        return false;
    };
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    let fmt = if hourly { "%Y-%m-%d %H:00" } else { "%Y-%m-%d" };
    let mtime = DateTime::<Utc>::from(modified)
        .with_timezone(&Local)
        .format(fmt)
        .to_string();
    mtime.as_str() < cutoff.as_str()
}

/// Zcode rollout 行: { model: { modelId }, completedAt, response: { usage: { inputTokens, ... } } }
fn parse_zcode_line(v: &serde_json::Value) -> Option<UsageRecord> {
    let usage = v.get("response")?.get("usage")?;
    let model = v.get("model")?.get("modelId")?.as_str()?.to_string();
    let date = v.get("completedAt")?.as_str()?.to_string();
    Some(UsageRecord {
        date,
        model,
        usage: UsageBucket {
            input: usage
                .get("inputTokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            output: usage
                .get("outputTokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            cache_read: usage
                .get("cacheReadTokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            cache_write: 0,
            reasoning: usage
                .get("reasoningTokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            requests: 1,
        },
    })
}

/// Claude Code 行: { timestamp, message: { model, usage: { input_tokens, ... } } }
fn parse_claude_line(v: &serde_json::Value) -> Option<UsageRecord> {
    let msg = v.get("message")?;
    let usage = msg.get("usage")?;
    let model = msg.get("model")?.as_str()?.to_string();
    let date = v.get("timestamp")?.as_str()?.to_string();
    Some(UsageRecord {
        date,
        model,
        usage: UsageBucket {
            input: usage
                .get("input_tokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            output: usage
                .get("output_tokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            cache_read: usage
                .get("cache_read_input_tokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            cache_write: usage
                .get("cache_creation_input_tokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            reasoning: 0,
            requests: 1,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn process_file(
    path: &Path,
    cutoff: &Option<String>,
    hourly: bool,
    parse_line: fn(&serde_json::Value) -> Option<UsageRecord>,
    source: &str,
    by_day: &mut HashMap<String, DayStats>,
    by_model: &mut HashMap<(String, String), UsageBucket>,
    totals: &mut UsageBucket,
) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(mut rec) = parse_line(&v) else {
            continue;
        };
        // 全零记录是流式占位行，跳过
        if rec.usage.total() <= 0 || rec.model.is_empty() {
            continue;
        }
        let Some(key) = local_key(&rec.date, hourly) else {
            continue;
        };
        rec.date = key;
        if let Some(c) = cutoff {
            if rec.date.as_str() < c.as_str() {
                continue;
            }
        }

        let day = by_day.entry(rec.date).or_default();
        let bucket = if source == "zcode" {
            &mut day.zcode
        } else {
            &mut day.claude
        };
        merge_bucket(bucket, &rec.usage);
        merge_bucket(totals, &rec.usage);
        merge_bucket(
            by_model
                .entry((source.to_string(), rec.model.clone()))
                .or_default(),
            &rec.usage,
        );
    }
}

fn merge_bucket(dst: &mut UsageBucket, src: &UsageBucket) {
    dst.input += src.input;
    dst.output += src.output;
    dst.cache_read += src.cache_read;
    dst.cache_write += src.cache_write;
    dst.reasoning += src.reasoning;
    dst.requests += src.requests;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zcode_line_extracts_usage() {
        let v = serde_json::json!({
            "completedAt": "2026-09-07T22:20:03.405Z",
            "model": { "modelId": "GLM-5.3" },
            "response": { "usage": { "inputTokens": 100, "outputTokens": 5, "cacheReadTokens": 50, "reasoningTokens": 10 } }
        });
        let rec = parse_zcode_line(&v).unwrap();
        assert_eq!(rec.model, "GLM-5.3");
        assert_eq!(rec.usage.input, 100);
        assert_eq!(rec.usage.output, 5);
        assert_eq!(rec.usage.cache_read, 50);
        assert_eq!(rec.usage.reasoning, 10);
        assert_eq!(rec.usage.total(), 165);
        assert_eq!(rec.date, "2026-09-07T22:20:03.405Z");
        // 时间档 key：按天 / 按小时
        assert!(local_key(&rec.date, false).unwrap().starts_with("2026-09-0"));
        assert!(local_key(&rec.date, true).unwrap().contains(":00"));
    }

    #[test]
    fn parse_claude_line_extracts_usage() {
        let v = serde_json::json!({
            "timestamp": "2026-08-31T23:46:33.845Z",
            "message": {
                "model": "k3-256k",
                "usage": { "input_tokens": 3539, "output_tokens": 413, "cache_read_input_tokens": 19456, "cache_creation_input_tokens": 7 }
            }
        });
        let rec = parse_claude_line(&v).unwrap();
        assert_eq!(rec.model, "k3-256k");
        assert_eq!(rec.usage.input, 3539);
        assert_eq!(rec.usage.output, 413);
        assert_eq!(rec.usage.cache_read, 19456);
        assert_eq!(rec.usage.cache_write, 7);
        assert_eq!(rec.date, "2026-08-31T23:46:33.845Z");
    }

    #[test]
    fn parse_line_without_usage_returns_none() {
        let v = serde_json::json!({ "type": "user", "message": { "content": "hi" } });
        assert!(parse_claude_line(&v).is_none());

        let v = serde_json::json!({ "completedAt": "2026-09-07T22:20:03.405Z" });
        assert!(parse_zcode_line(&v).is_none());
    }

    #[test]
    fn next_date_rolls_over() {
        assert_eq!(next_date("2026-09-07"), "2026-09-08");
        assert_eq!(next_date("2026-01-31"), "2026-02-01");
        assert_eq!(next_date("bad-date"), "bad-date");
    }

    #[test]
    fn next_hour_rolls_over() {
        assert_eq!(next_hour("2026-09-08 05:00"), "2026-09-08 06:00");
        assert_eq!(next_hour("2026-09-08 23:00"), "2026-09-09 00:00");
        assert_eq!(next_hour("bad"), "bad");
    }
}
