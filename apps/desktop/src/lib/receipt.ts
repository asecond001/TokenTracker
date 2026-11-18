import { convertMicroUsd, formatCost, formatExactInt, formatTokens } from './format';
import type { CurrencyCode, MonthlyReport } from '../types/ipc';

/** One printed line of the usage receipt. */
export type ReceiptLine =
  | { kind: 'title'; text: string }
  | { kind: 'subtitle'; text: string }
  | { kind: 'divider' }
  | { kind: 'header'; left: string; mid: string; right: string }
  | { kind: 'item'; left: string; mid: string; right: string }
  | { kind: 'row'; left: string; right: string; strong?: boolean }
  | { kind: 'note'; text: string }
  | { kind: 'barcode'; seed: string };

export interface ReceiptModel {
  lines: ReceiptLine[];
  fileName: string;
}

const MAX_ITEMS = 8;
/** Reference latte price used for the playful "≈ N 杯拿铁" conversion. */
const LATTE_USD = 4.5;
const LATTE_CNY = 30;

function pad2(n: number): string {
  return String(n).padStart(2, '0');
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function receiptNo(report: MonthlyReport): string {
  // Deterministic, non-identifying serial derived from the aggregates only.
  let h = 2166136261;
  const seed = `${report.year}-${report.month}-${report.month_total_tokens}-${report.month_total_requests}`;
  for (let i = 0; i < seed.length; i += 1) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return `TV${report.year}${pad2(report.month)}-${(h >>> 0).toString(16).toUpperCase().padStart(8, '0').slice(0, 6)}`;
}

function longestActiveStreak(report: MonthlyReport): number {
  let best = 0;
  let run = 0;
  const days = [...report.daily_ledger].sort((a, b) => a.date.localeCompare(b.date));
  let prev: number | null = null;
  for (const day of days) {
    if (day.total_tokens <= 0) {
      run = 0;
      prev = null;
      continue;
    }
    const t = new Date(`${day.date}T00:00:00`).getTime();
    run = prev !== null && Math.round((t - prev) / 86_400_000) === 1 ? run + 1 : 1;
    prev = t;
    best = Math.max(best, run);
  }
  return best;
}

export function buildReceiptModel(
  report: MonthlyReport,
  currency: CurrencyCode,
  usdToCnyRate: string,
  now: Date = new Date()
): ReceiptModel {
  const cost = (micro: number) => formatCost(micro, currency, usdToCnyRate);
  const period = `${report.year}-${pad2(report.month)}`;
  const lines: ReceiptLine[] = [
    { kind: 'title', text: 'TOKENVANE' },
    { kind: 'subtitle', text: `AI 用量小票 · ${period}` },
    { kind: 'divider' },
    { kind: 'row', left: '单号', right: receiptNo(report) },
    {
      kind: 'row',
      left: '打印',
      right: `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())} ${pad2(now.getHours())}:${pad2(now.getMinutes())}`,
    },
    { kind: 'divider' },
    { kind: 'header', left: '品名', mid: 'Token', right: '金额' },
  ];

  const models = [...(report.model_shares ?? [])]
    .filter((m) => m.tokens > 0 || m.cost_micro_usd > 0)
    .sort((a, b) => b.cost_micro_usd - a.cost_micro_usd || b.tokens - a.tokens);
  const shown = models.slice(0, MAX_ITEMS);
  for (const m of shown) {
    lines.push({
      kind: 'item',
      left: truncate(m.label || m.key, 22),
      mid: formatTokens(m.tokens),
      right: cost(m.cost_micro_usd),
    });
  }
  const rest = models.slice(MAX_ITEMS);
  if (rest.length > 0) {
    lines.push({
      kind: 'item',
      left: `其他 ${rest.length} 个模型`,
      mid: formatTokens(rest.reduce((acc, m) => acc + m.tokens, 0)),
      right: cost(rest.reduce((acc, m) => acc + m.cost_micro_usd, 0)),
    });
  }
  if (models.length === 0) {
    lines.push({ kind: 'note', text: '本月等待第一笔消费' });
  }

  lines.push(
    { kind: 'divider' },
    { kind: 'row', left: '请求次数', right: formatExactInt(report.month_total_requests) },
    { kind: 'row', left: 'Token 合计', right: formatTokens(report.month_total_tokens) },
    { kind: 'row', left: '缓存立减', right: `-${cost(report.month_cache_saved_micro_usd)}` },
    { kind: 'divider' },
    { kind: 'row', left: 'API 等效合计', right: cost(report.api_equivalent_cost_micro_usd), strong: true }
  );
  if (report.subscription_monthly_fee_micro_usd > 0) {
    lines.push({ kind: 'row', left: '订阅实付', right: cost(report.subscription_monthly_fee_micro_usd) });
    if (report.roi_multiplier !== null && Number.isFinite(report.roi_multiplier)) {
      lines.push({ kind: 'row', left: '回本倍数', right: `${report.roi_multiplier.toFixed(1)}×`, strong: true });
    }
  }

  const busiest = [...report.daily_ledger].sort((a, b) => b.total_tokens - a.total_tokens)[0];
  const outputTokens = report.daily_ledger.reduce((acc, d) => acc + d.output_tokens, 0);
  const streak = longestActiveStreak(report);
  const spend = convertMicroUsd(report.api_equivalent_cost_micro_usd, currency, usdToCnyRate);
  const lattes = Math.floor(spend / (currency === 'CNY' ? LATTE_CNY : LATTE_USD));

  lines.push({ kind: 'divider' });
  if (busiest && busiest.total_tokens > 0) {
    lines.push({ kind: 'row', left: '最忙一天', right: `${busiest.date.slice(5)} · ${formatTokens(busiest.total_tokens)}` });
  }
  if (streak > 0) {
    lines.push({ kind: 'row', left: '连续活跃', right: `${streak} 天` });
  }
  if (outputTokens > 0) {
    // Rough rule of thumb: one output token ≈ 1.5 汉字.
    const wan = (outputTokens * 1.5) / 10_000;
    lines.push({ kind: 'row', left: 'AI 代笔', right: `≈ ${wan >= 100 ? Math.round(wan).toLocaleString('en-US') : wan.toFixed(1)} 万字` });
  }
  if (lattes > 0) {
    lines.push({ kind: 'row', left: '折合拿铁', right: `≈ ${lattes.toLocaleString('en-US')} 杯` });
  }

  lines.push(
    { kind: 'barcode', seed: receiptNo(report) },
    { kind: 'note', text: '谢谢惠顾 · 欢迎下月再来' }
  );

  return { lines, fileName: `tokenvane-receipt-${period}.png` };
}

export interface ReceiptPalette {
  paper: string;
  ink: string;
  muted: string;
  rule: string;
  backdrop: string;
}

export const RECEIPT_PALETTE: ReceiptPalette = {
  paper: '#fbfaf6',
  ink: '#1f2328',
  muted: '#6b7079',
  rule: '#b9bcc2',
  backdrop: 'transparent',
};

const FONT_STACK = "'Cascadia Mono', Consolas, 'Microsoft YaHei UI', 'PingFang SC', monospace";
const WIDTH = 360;
const PAD_X = 22;

function lineHeight(line: ReceiptLine): number {
  switch (line.kind) {
    case 'title':
      return 34;
    case 'subtitle':
      return 22;
    case 'divider':
      return 16;
    case 'barcode':
      return 62;
    default:
      return 20;
  }
}

/** Renders the receipt to a canvas at `scale` device pixels per CSS pixel. */
export function drawReceipt(
  canvas: HTMLCanvasElement,
  model: ReceiptModel,
  scale = 2,
  palette: ReceiptPalette = RECEIPT_PALETTE
): void {
  const tooth = 8;
  const contentHeight = model.lines.reduce((acc, l) => acc + lineHeight(l), 0);
  const height = contentHeight + 28 + tooth * 2;
  canvas.width = WIDTH * scale;
  canvas.height = height * scale;
  canvas.style.width = `${WIDTH}px`;
  canvas.style.height = `${height}px`;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, WIDTH, height);

  // Paper with zig-zag tear edges top and bottom.
  ctx.fillStyle = palette.paper;
  ctx.beginPath();
  ctx.moveTo(0, tooth);
  for (let x = 0; x <= WIDTH; x += tooth * 2) {
    ctx.lineTo(x + tooth, 0);
    ctx.lineTo(Math.min(WIDTH, x + tooth * 2), tooth);
  }
  ctx.lineTo(WIDTH, height - tooth);
  for (let x = WIDTH; x >= 0; x -= tooth * 2) {
    ctx.lineTo(x - tooth, height);
    ctx.lineTo(Math.max(0, x - tooth * 2), height - tooth);
  }
  ctx.closePath();
  ctx.fill();

  let y = tooth + 14;
  const right = WIDTH - PAD_X;
  const midX = WIDTH * 0.66;
  ctx.textBaseline = 'middle';

  for (const line of model.lines) {
    const h = lineHeight(line);
    const cy = y + h / 2;
    switch (line.kind) {
      case 'title':
        ctx.fillStyle = palette.ink;
        ctx.font = `700 22px ${FONT_STACK}`;
        ctx.textAlign = 'center';
        ctx.fillText(line.text.split('').join(' '), WIDTH / 2, cy);
        break;
      case 'subtitle':
        ctx.fillStyle = palette.muted;
        ctx.font = `12px ${FONT_STACK}`;
        ctx.textAlign = 'center';
        ctx.fillText(line.text, WIDTH / 2, cy);
        break;
      case 'divider':
        ctx.strokeStyle = palette.rule;
        ctx.lineWidth = 1;
        ctx.setLineDash([4, 3]);
        ctx.beginPath();
        ctx.moveTo(PAD_X, cy + 0.5);
        ctx.lineTo(right, cy + 0.5);
        ctx.stroke();
        ctx.setLineDash([]);
        break;
      case 'header':
      case 'item':
        ctx.fillStyle = line.kind === 'header' ? palette.muted : palette.ink;
        ctx.font = `${line.kind === 'header' ? 11 : 12}px ${FONT_STACK}`;
        ctx.textAlign = 'left';
        ctx.fillText(line.left, PAD_X, cy);
        ctx.textAlign = 'right';
        ctx.fillText(line.mid, midX, cy);
        ctx.fillText(line.right, right, cy);
        break;
      case 'row':
        ctx.fillStyle = line.strong ? palette.ink : palette.muted;
        ctx.font = `${line.strong ? 700 : 400} ${line.strong ? 13 : 12}px ${FONT_STACK}`;
        ctx.textAlign = 'left';
        ctx.fillText(line.left, PAD_X, cy);
        ctx.fillStyle = palette.ink;
        ctx.textAlign = 'right';
        ctx.fillText(line.right, right, cy);
        break;
      case 'note':
        ctx.fillStyle = palette.muted;
        ctx.font = `11.5px ${FONT_STACK}`;
        ctx.textAlign = 'center';
        ctx.fillText(line.text, WIDTH / 2, cy);
        break;
      case 'barcode': {
        // Decorative barcode derived from the receipt serial.
        ctx.fillStyle = palette.ink;
        const barTop = y + 10;
        const barH = 34;
        let x = PAD_X + 24;
        const end = right - 24;
        let i = 0;
        while (x < end) {
          const code = line.seed.charCodeAt(i % line.seed.length) + i * 7;
          const w = (code % 3) + 1;
          if (i % 2 === 0) ctx.fillRect(x, barTop, w, barH);
          x += w + ((code >> 2) % 2) + 1;
          i += 1;
        }
        ctx.fillStyle = palette.muted;
        ctx.font = `10px ${FONT_STACK}`;
        ctx.textAlign = 'center';
        ctx.fillText(line.seed, WIDTH / 2, barTop + barH + 10);
        break;
      }
    }
    y += h;
  }
}
