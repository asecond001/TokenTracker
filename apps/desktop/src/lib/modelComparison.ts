import type {
  ModelStatRow,
  ModelStatsReport,
  SpeedLatencyReport,
} from '../types/ipc';

/** One model (per provider) with price and speed joined side by side. */
export interface ModelComparisonRow {
  key: string;
  model_id: string;
  provider_id: string;
  requests: number;
  total_tokens: number;
  output_tokens: number;
  cost_micro_usd: number;
  cache_saved_micro_usd: number;
  /** Blended API-equivalent cost per 1M tokens, in micro-USD. `null` without tokens. */
  cost_per_m_micro_usd: number | null;
  /** List price per 1M output tokens from the effective pricing entry. */
  output_price_per_m_micro_usd: number | null;
  cache_hit_ratio: number | null;
  mean_tps: number | null;
  p95_tps: number | null;
  avg_ttft_ms: number | null;
  avg_duration_ms: number | null;
  speed_samples: number;
}

/** Provider-level roll-up of the same metrics. */
export interface ProviderComparisonRow {
  provider_id: string;
  model_count: number;
  requests: number;
  total_tokens: number;
  cost_micro_usd: number;
  cost_share: number;
  cache_saved_micro_usd: number;
  cost_per_m_micro_usd: number | null;
  cache_hit_ratio: number | null;
  mean_tps: number | null;
  p95_tps: number | null;
  avg_ttft_ms: number | null;
  speed_samples: number;
}

function costPerMillion(costMicro: number, tokens: number): number | null {
  return tokens > 0 ? (costMicro / tokens) * 1_000_000 : null;
}

function cacheHitRatio(row: Pick<ModelStatRow, 'input_tokens' | 'cache_read_tokens'>): number | null {
  const denom = row.input_tokens + row.cache_read_tokens;
  return denom > 0 ? row.cache_read_tokens / denom : null;
}

function positiveOrNull(v: number | null | undefined): number | null {
  return typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null;
}

export function buildModelComparison(
  models: ModelStatsReport | undefined,
  speed: SpeedLatencyReport | undefined
): ModelComparisonRow[] {
  const speedByKey = new Map<string, SpeedLatencyReport['models'][number]>(
    (speed?.models ?? []).map((m) => [`${m.provider_id}::${m.model_id}`, m] as const)
  );
  const rows = new Map<string, ModelComparisonRow>();

  for (const m of models?.rows ?? []) {
    const key = `${m.provider_id}::${m.model_id}`;
    const s = speedByKey.get(key);
    rows.set(key, {
      key,
      model_id: m.model_id,
      provider_id: m.provider_id,
      requests: m.requests,
      total_tokens: m.total_tokens,
      output_tokens: m.output_tokens + m.reasoning_tokens,
      cost_micro_usd: m.estimated_cost_micro_usd,
      cache_saved_micro_usd: m.cache_saved_micro_usd,
      cost_per_m_micro_usd: costPerMillion(m.estimated_cost_micro_usd, m.total_tokens),
      output_price_per_m_micro_usd: positiveOrNull(m.effective_pricing?.output_per_m_micro_usd),
      cache_hit_ratio: cacheHitRatio(m),
      mean_tps: positiveOrNull(s?.mean_tps ?? m.avg_output_tps),
      p95_tps: positiveOrNull(s?.p95_tps),
      avg_ttft_ms: positiveOrNull(s?.avg_ttft_ms),
      avg_duration_ms: positiveOrNull(s?.avg_duration_ms),
      speed_samples: s?.sample_count ?? 0,
    });
  }

  // Models with speed samples but no billable rows in range still get a row.
  for (const [key, s] of speedByKey) {
    if (rows.has(key)) continue;
    rows.set(key, {
      key,
      model_id: s.model_id,
      provider_id: s.provider_id,
      requests: 0,
      total_tokens: 0,
      output_tokens: 0,
      cost_micro_usd: 0,
      cache_saved_micro_usd: 0,
      cost_per_m_micro_usd: null,
      output_price_per_m_micro_usd: null,
      cache_hit_ratio: null,
      mean_tps: positiveOrNull(s.mean_tps),
      p95_tps: positiveOrNull(s.p95_tps),
      avg_ttft_ms: positiveOrNull(s.avg_ttft_ms),
      avg_duration_ms: positiveOrNull(s.avg_duration_ms),
      speed_samples: s.sample_count,
    });
  }

  return [...rows.values()].sort(
    (a, b) => b.cost_micro_usd - a.cost_micro_usd || b.total_tokens - a.total_tokens
  );
}

export function buildProviderComparison(
  rows: ModelComparisonRow[],
  speed: SpeedLatencyReport | undefined,
  models: ModelStatsReport | undefined
): ProviderComparisonRow[] {
  const speedByProvider = new Map((speed?.providers ?? []).map((p) => [p.provider_id, p] as const));
  const totalCost = rows.reduce((acc, r) => acc + r.cost_micro_usd, 0);
  const cacheByProvider = new Map<string, { input: number; cacheRead: number }>();
  for (const m of models?.rows ?? []) {
    const e = cacheByProvider.get(m.provider_id) ?? { input: 0, cacheRead: 0 };
    e.input += m.input_tokens;
    e.cacheRead += m.cache_read_tokens;
    cacheByProvider.set(m.provider_id, e);
  }

  const grouped = new Map<string, ProviderComparisonRow>();
  for (const r of rows) {
    const g = grouped.get(r.provider_id) ?? {
      provider_id: r.provider_id,
      model_count: 0,
      requests: 0,
      total_tokens: 0,
      cost_micro_usd: 0,
      cost_share: 0,
      cache_saved_micro_usd: 0,
      cost_per_m_micro_usd: null,
      cache_hit_ratio: null,
      mean_tps: null,
      p95_tps: null,
      avg_ttft_ms: null,
      speed_samples: 0,
    };
    g.model_count += 1;
    g.requests += r.requests;
    g.total_tokens += r.total_tokens;
    g.cost_micro_usd += r.cost_micro_usd;
    g.cache_saved_micro_usd += r.cache_saved_micro_usd;
    grouped.set(r.provider_id, g);
  }

  for (const g of grouped.values()) {
    const s = speedByProvider.get(g.provider_id);
    const c = cacheByProvider.get(g.provider_id);
    g.cost_share = totalCost > 0 ? g.cost_micro_usd / totalCost : 0;
    g.cost_per_m_micro_usd = costPerMillion(g.cost_micro_usd, g.total_tokens);
    g.cache_hit_ratio = c && c.input + c.cacheRead > 0 ? c.cacheRead / (c.input + c.cacheRead) : null;
    g.mean_tps = positiveOrNull(s?.mean_tps);
    g.p95_tps = positiveOrNull(s?.p95_tps);
    g.avg_ttft_ms = positiveOrNull(s?.avg_ttft_ms);
    g.speed_samples = s?.sample_count ?? 0;
  }

  return [...grouped.values()].sort(
    (a, b) => b.cost_micro_usd - a.cost_micro_usd || b.total_tokens - a.total_tokens
  );
}

export type ComparisonSortKey =
  | 'cost_micro_usd'
  | 'total_tokens'
  | 'cost_per_m_micro_usd'
  | 'mean_tps'
  | 'avg_ttft_ms'
  | 'cache_hit_ratio';

/** Sorts with `null` values always last, regardless of direction. */
export function sortComparisonRows<T extends Record<ComparisonSortKey, number | null>>(
  rows: T[],
  key: ComparisonSortKey,
  desc: boolean
): T[] {
  return [...rows].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    if (av === null && bv === null) return 0;
    if (av === null) return 1;
    if (bv === null) return -1;
    return desc ? bv - av : av - bv;
  });
}
