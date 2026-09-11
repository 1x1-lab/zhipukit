use crate::types::{DayStats, ModelStats, TokenStatsResult, UsageBucket};
use crate::utils::get_home_dir;
use chrono::{DateTime, Local, Utc};
use rusqlite::OpenFlags;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// 从一行日志解析出的单次用量记录（date 为原始 ISO 时间戳，由调用方按粒度聚合）
struct UsageRecord {
    date: String,
    model: String,
    /// 供应商 ID（仅 Zcode 有，如 "builtin:bigmodel" 或自定义 UUID）
    provider: String,
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
    let zcode_db = home.join(".zcode").join("cli").join("db").join("db.sqlite");
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
        zcode_detected: zcode_db.exists() || zcode_dir.exists(),
        claude_detected: claude_dir.exists(),
        granularity: if hourly { "hour" } else { "day" }.to_string(),
        ..Default::default()
    };
    let mut by_day: HashMap<String, DayStats> = HashMap::new();
    let mut by_model: HashMap<(String, String, String), UsageBucket> = HashMap::new(); // (source, provider, model)
    let provider_names = zcode_provider_names();

    // Zcode 首选 sqlite 数据库（权威数据源，rollout jsonl 只含少量请求样本）；
    // 数据库不存在或读取失败时回退 jsonl，两者记录重叠，不可同时统计
    let mut zcode_from_db = false;
    if zcode_db.exists() {
        match scan_zcode_db(
            &zcode_db,
            &cutoff,
            hourly,
            &mut by_day,
            &mut by_model,
            &mut result.totals_zcode,
            &mut result.zcode_sessions,
        ) {
            Ok(()) => zcode_from_db = true,
            Err(e) => log::warn!("读取 Zcode 用量数据库失败({})，回退 rollout jsonl 扫描", e),
        }
    }

    if !zcode_from_db {
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
    result.by_day = dates
        .into_iter()
        .filter_map(|d| by_day.remove(&d))
        .map(|mut d| {
            // 当日模型明细同样按总 token 降序
            d.models
                .sort_by(|a, b| b.usage.total().cmp(&a.usage.total()));
            d
        })
        .collect();

    // 按总 token 降序输出模型聚合（provider 显示为可读名称，未知名回退原始 ID）
    let mut models: Vec<ModelStats> = by_model
        .into_iter()
        .map(|((source, provider, model), usage)| ModelStats {
            model,
            source,
            provider: provider_names.get(&provider).cloned().unwrap_or(provider),
            usage,
        })
        .collect();
    models.sort_by(|a, b| b.usage.total().cmp(&a.usage.total()));
    result.by_model = models;

    Ok(result)
}

/// ISO 时间戳转本地时区时间档 key（按天 "YYYY-MM-DD" 或按小时 "YYYY-MM-DD HH:00"）
fn local_key(ts: &str, hourly: bool) -> Option<String> {
    DateTime::parse_from_rfc3339(ts).ok().map(|dt| {
        let local = dt.with_timezone(&Local);
        if hourly {
            local.format("%Y-%m-%d %H:00").to_string()
        } else {
            local.format("%Y-%m-%d").to_string()
        }
    })
}

/// 读取 Zcode config.json 的 provider id → 可读名称映射
fn zcode_provider_names() -> HashMap<String, String> {
    let Ok(home) = get_home_dir() else {
        return HashMap::new();
    };
    let path = home.join(".zcode").join("v2").join("config.json");
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<serde_json::Value>(&content) else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    if let Some(providers) = raw.get("provider").and_then(|p| p.as_object()) {
        for (id, val) in providers {
            if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                if !name.is_empty() {
                    map.insert(id.clone(), name.to_string());
                }
            }
        }
    }
    map
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

/// Zcode rollout 行: { model: { modelId, providerId }, completedAt, response: { usage: { inputTokens, ... } } }
fn parse_zcode_line(v: &serde_json::Value) -> Option<UsageRecord> {
    let usage = v.get("response")?.get("usage")?;
    let model = v.get("model")?;
    let model_id = model.get("modelId")?.as_str()?.to_string();
    let provider = model
        .get("providerId")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let date = v.get("completedAt")?.as_str()?.to_string();
    Some(UsageRecord {
        date,
        model: model_id,
        provider,
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
        provider: String::new(),
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

/// 从 db.sqlite 的 model_usage 表聚合 Zcode 用量（权威数据源）
#[allow(clippy::too_many_arguments)]
fn scan_zcode_db(
    db_path: &Path,
    cutoff: &Option<String>,
    hourly: bool,
    by_day: &mut HashMap<String, DayStats>,
    by_model: &mut HashMap<(String, String, String), UsageBucket>,
    totals: &mut UsageBucket,
    sessions: &mut i64,
) -> Result<(), String> {
    // 只读打开；WAL 库在 -shm 文件缺失时只读打开可能失败，退回读写打开（仅查询，不写入）
    let conn = rusqlite::Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .or_else(|_| {
            rusqlite::Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        })
        .map_err(|e| format!("打开数据库失败: {}", e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(1000));

    let mut stmt = conn
        .prepare(
            "SELECT started_at, session_id, provider_id, model_id, \
             input_tokens, output_tokens, reasoning_tokens, \
             cache_creation_input_tokens, cache_read_input_tokens \
             FROM model_usage WHERE status = 'completed'",
        )
        .map_err(|e| format!("查询 model_usage 失败: {}", e))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| format!("查询 model_usage 失败: {}", e))?;

    let mut seen_sessions: HashSet<String> = HashSet::new();
    while let Some(row) = rows.next().map_err(|e| format!("读取记录失败: {}", e))? {
        let started_ms: i64 = row.get(0).map_err(|e| format!("读取记录失败: {}", e))?;
        let session_id: String = row.get(1).unwrap_or_default();
        let provider: String = row.get(2).unwrap_or_default();
        let model: String = row.get(3).unwrap_or_default();
        let input: i64 = row.get(4).unwrap_or(0);
        let output: i64 = row.get(5).unwrap_or(0);
        let reasoning: i64 = row.get(6).unwrap_or(0);
        let cache_write: i64 = row.get(7).unwrap_or(0);
        let cache_read: i64 = row.get(8).unwrap_or(0);
        let Some(dt) = DateTime::<Utc>::from_timestamp_millis(started_ms) else {
            continue;
        };
        let merged = merge_record(
            UsageRecord {
                date: dt.to_rfc3339(),
                model,
                provider,
                usage: UsageBucket {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    reasoning,
                    requests: 1,
                },
            },
            cutoff,
            hourly,
            "zcode",
            by_day,
            by_model,
            totals,
        );
        if merged {
            seen_sessions.insert(session_id);
        }
    }
    *sessions = seen_sessions.len() as i64;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_file(
    path: &Path,
    cutoff: &Option<String>,
    hourly: bool,
    parse_line: fn(&serde_json::Value) -> Option<UsageRecord>,
    source: &str,
    by_day: &mut HashMap<String, DayStats>,
    by_model: &mut HashMap<(String, String, String), UsageBucket>,
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
        let Some(rec) = parse_line(&v) else {
            continue;
        };
        merge_record(rec, cutoff, hourly, source, by_day, by_model, totals);
    }
}

/// 将单条用量记录聚合进时间档/模型/总计；返回是否被计入（调用方用于会话计数）
#[allow(clippy::too_many_arguments)]
fn merge_record(
    rec: UsageRecord,
    cutoff: &Option<String>,
    hourly: bool,
    source: &str,
    by_day: &mut HashMap<String, DayStats>,
    by_model: &mut HashMap<(String, String, String), UsageBucket>,
    totals: &mut UsageBucket,
) -> bool {
    // 全零记录是流式占位行，跳过
    if rec.usage.total() <= 0 || rec.model.is_empty() {
        return false;
    }
    let Some(date) = local_key(&rec.date, hourly) else {
        return false;
    };
    if let Some(c) = cutoff {
        if date.as_str() < c.as_str() {
            return false;
        }
    }
    let UsageRecord {
        model,
        provider,
        usage,
        ..
    } = rec;

    let day = by_day.entry(date).or_default();
    let bucket = if source == "zcode" {
        &mut day.zcode
    } else {
        &mut day.claude
    };
    merge_bucket(bucket, &usage);
    merge_bucket(totals, &usage);
    merge_bucket(
        by_model
            .entry((source.to_string(), provider.clone(), model.clone()))
            .or_default(),
        &usage,
    );

    // 当日按模型明细（前端按模型过滤时间轴用）
    match day
        .models
        .iter_mut()
        .find(|m| m.model == model && m.provider == provider && m.source == source)
    {
        Some(m) => merge_bucket(&mut m.usage, &usage),
        None => day.models.push(ModelStats {
            model,
            provider,
            source: source.to_string(),
            usage,
        }),
    }
    true
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
            "model": { "modelId": "GLM-5.3", "providerId": "builtin:bigmodel" },
            "response": { "usage": { "inputTokens": 100, "outputTokens": 5, "cacheReadTokens": 50, "reasoningTokens": 10 } }
        });
        let rec = parse_zcode_line(&v).unwrap();
        assert_eq!(rec.model, "GLM-5.3");
        assert_eq!(rec.provider, "builtin:bigmodel");
        assert_eq!(rec.usage.input, 100);
        assert_eq!(rec.usage.output, 5);
        assert_eq!(rec.usage.cache_read, 50);
        assert_eq!(rec.usage.reasoning, 10);
        assert_eq!(rec.usage.total(), 165);
        assert_eq!(rec.date, "2026-09-07T22:20:03.405Z");
        // 时间档 key：按天 / 按小时
        assert!(local_key(&rec.date, false)
            .unwrap()
            .starts_with("2026-09-0"));
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

    #[test]
    fn scan_zcode_db_aggregates_model_usage() {
        let db = std::env::temp_dir().join(format!(
            "token-tool-usage-test-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db);
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE model_usage (
                    id text primary key,
                    session_id text not null,
                    provider_id text not null,
                    model_id text not null,
                    status text not null,
                    started_at integer not null,
                    input_tokens integer not null default 0,
                    output_tokens integer not null default 0,
                    reasoning_tokens integer not null default 0,
                    cache_creation_input_tokens integer not null default 0,
                    cache_read_input_tokens integer not null default 0
                );",
            )
            .unwrap();
            let now_ms = Utc::now().timestamp_millis();
            let insert = |id: &str, session: &str, status: &str, tokens: [i64; 5]| {
                conn.execute(
                    "INSERT INTO model_usage (id, session_id, provider_id, model_id, status, \
                     started_at, input_tokens, output_tokens, reasoning_tokens, \
                     cache_creation_input_tokens, cache_read_input_tokens) \
                     VALUES (?1, ?2, 'builtin:bigmodel', 'GLM-5.3', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        id, session, status, now_ms, tokens[0], tokens[1], tokens[2], tokens[3],
                        tokens[4]
                    ],
                )
                .unwrap();
            };
            insert("r1", "s1", "completed", [100, 5, 10, 0, 50]);
            insert("r2", "s1", "completed", [200, 7, 0, 0, 0]);
            // error 状态与全零记录不计入
            insert("r3", "s2", "error", [999, 999, 0, 0, 0]);
            insert("r4", "s2", "completed", [0, 0, 0, 0, 0]);
        }

        let mut by_day = HashMap::new();
        let mut by_model = HashMap::new();
        let mut totals = UsageBucket::default();
        let mut sessions = 0i64;
        scan_zcode_db(
            &db,
            &None,
            false,
            &mut by_day,
            &mut by_model,
            &mut totals,
            &mut sessions,
        )
        .unwrap();

        assert_eq!(totals.input, 300);
        assert_eq!(totals.output, 12);
        assert_eq!(totals.cache_read, 50);
        assert_eq!(totals.reasoning, 10);
        assert_eq!(totals.requests, 2);
        assert_eq!(sessions, 1); // 只有 s1 有计入的记录
        assert_eq!(by_model.len(), 1);
        assert_eq!(by_day.len(), 1);

        // 当日按模型明细：两笔合并到同一模型
        let day = by_day.values().next().unwrap();
        assert_eq!(day.zcode.input, 300);
        assert_eq!(day.models.len(), 1);
        assert_eq!(day.models[0].model, "GLM-5.3");
        assert_eq!(day.models[0].provider, "builtin:bigmodel");
        assert_eq!(day.models[0].usage.input, 300);
        assert_eq!(day.models[0].usage.requests, 2);

        let _ = std::fs::remove_file(&db);
    }
}
