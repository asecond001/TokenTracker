//! Shared SQL scope builders, time-window math and period totals.

use super::*;

#[derive(Default)]
pub(super) struct PeriodTotals {
    pub(super) requests: i64,
    pub(super) observable_steps: i64,
    pub(super) input: i64,
    pub(super) output: i64,
    pub(super) cache_read: i64,
    pub(super) cache_write: i64,
    pub(super) reasoning: i64,
    pub(super) cost_micro_usd: i64,
    pub(super) cache_saved_micro_usd: i64,
    pub(super) tps_count: i64,
    pub(super) tps_sum: f64,
    pub(super) max_tps: f64,
}

impl PeriodTotals {
    pub(super) fn total_tokens(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write + self.reasoning
    }

    pub(super) fn cache_hit_ratio(&self) -> f64 {
        let denom = self.input + self.cache_read + self.cache_write;
        if denom <= 0 {
            0.0
        } else {
            self.cache_read as f64 / denom as f64
        }
    }

    pub(super) fn avg_tps(&self) -> f64 {
        if self.tps_count <= 0 {
            0.0
        } else {
            self.tps_sum / self.tps_count as f64
        }
    }
}

pub(super) fn normalized_optional_filter(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
}

pub(super) struct SqlScopeClause {
    pub(super) sql_and: String,
    pub(super) sql_where: String,
    pub(super) params: Vec<Value>,
}

pub(super) fn build_scope_clause(
    base_param_count: usize,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> SqlScopeClause {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    let mut next_idx = base_param_count + 1;
    if let Some(prov) = provider {
        parts.push(format!("provider_id = ?{next_idx}"));
        params.push(Value::Text(prov.to_string()));
        next_idx += 1;
    }
    if let Some(dev) = device_id {
        parts.push(format!("device_id = ?{next_idx}"));
        params.push(Value::Text(dev.to_string()));
    }
    if parts.is_empty() {
        SqlScopeClause {
            sql_and: String::new(),
            sql_where: String::new(),
            params,
        }
    } else {
        let joined = parts.join(" AND ");
        SqlScopeClause {
            sql_and: format!(" AND {joined}"),
            sql_where: format!(" WHERE {joined}"),
            params,
        }
    }
}

pub(super) fn bound_range_params(start_ms: i64, end_ms: i64, extra: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(2 + extra.len());
    out.push(Value::Integer(start_ms));
    out.push(Value::Integer(end_ms));
    out.extend_from_slice(extra);
    out
}

pub(super) fn short_session_key(provider_id: &str, session_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b":");
    hasher.update(session_id.as_bytes());
    let digest = hasher.finalize();
    format!("#{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2])
}

pub(super) fn sanitize_project_label(raw: &str) -> String {
    let leaf = raw
        .trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim();
    if leaf.is_empty() {
        "workspace".to_string()
    } else {
        leaf.to_string()
    }
}

pub(super) fn pct_share(part: i64, total: i64) -> f64 {
    if total <= 0 || part <= 0 {
        0.0
    } else {
        (part as f64 / total as f64) * 100.0
    }
}

pub(super) fn pct_change(current: f64, previous: f64) -> Option<f64> {
    if previous.abs() < f64::EPSILON {
        None
    } else {
        Some(((current - previous) / previous) * 100.0)
    }
}

pub(super) fn local_offset(tz_offset_minutes: i32) -> FixedOffset {
    use chrono::Offset;
    FixedOffset::east_opt(tz_offset_minutes.clamp(-720, 840) * 60).unwrap_or_else(|| Utc.fix())
}

pub(super) fn local_day_start_ms(ts_ms: i64, tz_offset_minutes: i32) -> i64 {
    let offset_ms = (tz_offset_minutes.clamp(-720, 840) as i64) * 60_000;
    (ts_ms + offset_ms).div_euclid(DAY_MS) * DAY_MS - offset_ms
}

pub(super) fn local_week_start_ms(ts_ms: i64, tz_offset_minutes: i32) -> i64 {
    const WEEK_MS: i64 = 7 * DAY_MS;
    let offset_ms = (tz_offset_minutes.clamp(-720, 840) as i64) * 60_000;
    // 1970-01-01 UTC was a Thursday (+3 days after Monday 1969-12-29).
    let shifted = ts_ms + offset_ms + 3 * DAY_MS;
    shifted.div_euclid(WEEK_MS) * WEEK_MS - 3 * DAY_MS - offset_ms
}

pub(super) fn inclusive_window_span(start_ms: i64, end_ms: i64) -> i64 {
    let raw = end_ms.saturating_sub(start_ms);
    if end_ms.rem_euclid(1_000) == 999 {
        raw.saturating_add(1).max(HOUR_MS)
    } else {
        raw.max(HOUR_MS)
    }
}

pub(crate) fn local_day_bounds_ms(now_ms: i64, tz_offset_minutes: i32) -> (i64, i64) {
    let offset = local_offset(tz_offset_minutes);
    let dt = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&offset);
    let date = dt.date_naive();
    let start_dt = date
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| offset.from_local_datetime(&ndt).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_ms - DAY_MS);
    let month_start_date = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
    let month_start_ms = month_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| offset.from_local_datetime(&ndt).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(start_dt);
    (start_dt, month_start_ms)
}

pub(super) fn date_key_from_ms(ts_ms: i64, offset: FixedOffset) -> String {
    Utc.timestamp_millis_opt(ts_ms)
        .single()
        .map(|dt| dt.with_timezone(&offset).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

pub(super) fn query_period_totals(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<PeriodTotals> {
    let scope = build_scope_clause(2, provider, device_id);
    let sql = format!(
        r#"
        SELECT
            COALESCE(SUM(request_count), 0),
            COALESCE(SUM(observable_step_count), 0),
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(SUM(cache_read_tokens), 0),
            COALESCE(SUM(cache_write_tokens), 0),
            COALESCE(SUM(reasoning_tokens), 0),
            COALESCE(SUM(estimated_cost_micro_usd), 0),
            COALESCE(SUM(cache_saved_micro_usd), 0),
            COALESCE(SUM(tps_sample_count), 0),
            COALESCE(SUM(tps_sum), 0.0),
            COALESCE(MAX(max_output_tps), 0.0)
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
        "#,
        where_and = scope.sql_and
    );

    // Align lower bound down to hour bucket boundary so exact hour-bucketed rollups match event windows.
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let bound = bound_range_params(aligned_start, end_ms, &scope.params);
    conn.query_row(&sql, params_from_iter(bound), |row| {
        Ok(PeriodTotals {
            requests: row.get(0)?,
            observable_steps: row.get(1)?,
            input: row.get(2)?,
            output: row.get(3)?,
            cache_read: row.get(4)?,
            cache_write: row.get(5)?,
            reasoning: row.get(6)?,
            cost_micro_usd: row.get(7)?,
            cache_saved_micro_usd: row.get(8)?,
            tps_count: row.get(9)?,
            tps_sum: row.get(10)?,
            max_tps: row.get(11)?,
        })
    })
    .map_err(CoreError::from)
}

pub(super) fn resolve_effective_window(
    conn: &Connection,
    filter: &AnalyticsFilter,
    provider: Option<&str>,
    device_id: Option<&str>,
    fallback_now_ms: i64,
) -> CoreResult<(i64, i64)> {
    if !filter.all_time {
        let start = filter.start_ms.min(filter.end_ms);
        return Ok((start, filter.end_ms));
    }
    let end_ms = if filter.end_ms > 0 {
        filter.end_ms
    } else {
        fallback_now_ms
    };
    let scope = build_scope_clause(0, provider, device_id);
    let events_sql = format!(
        "SELECT MIN(timestamp_ms) FROM usage_events{where_clause}",
        where_clause = scope.sql_where
    );
    let events_min: Option<i64> =
        conn.query_row(&events_sql, params_from_iter(scope.params.clone()), |r| r.get(0))?;
    let rollups_sql = format!(
        "SELECT MIN(bucket_start_ms) FROM usage_hourly_rollups{where_clause}",
        where_clause = scope.sql_where
    );
    let rollups_min: Option<i64> =
        conn.query_row(&rollups_sql, params_from_iter(scope.params), |r| r.get(0))?;

    let min_ts = match (events_min, rollups_min) {
        (Some(e), Some(r)) => Some(e.min(r)),
        (Some(e), None) => Some(e),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    };
    let start_ms = min_ts
        .unwrap_or_else(|| end_ms.saturating_sub(7 * DAY_MS))
        .min(end_ms);
    Ok((start_ms, end_ms))
}

pub(super) fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub(super) fn format_dimension_label(dim: StackDimension, key: &str) -> String {
    match dim {
        StackDimension::Provider => provider_ids::display_name(key).to_string(),
        StackDimension::Model => key.to_string(),
        StackDimension::TokenType => match key {
            "input" => "Input".to_string(),
            "output" => "Output".to_string(),
            "cache_read" => "Cache Read".to_string(),
            "cache_write" => "Cache Write".to_string(),
            "reasoning" => "Reasoning".to_string(),
            other => other.to_string(),
        },
    }
}
