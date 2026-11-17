import { describe, expect, it } from 'vitest';
import {
  convertMicroUsd,
  formatCost,
  formatCountdown,
  formatPercent,
  formatTokens,
} from '../lib/format';

describe('TokenVane formatting & currency conversion', () => {
  it('formats token quantities concisely', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(950)).toBe('950');
    expect(formatTokens(12_500)).toBe('12.5K');
    expect(formatTokens(1_500_000)).toBe('1.50M');
    expect(formatTokens(2_250_000_000)).toBe('2.25B');
  });

  it('converts micro-USD accurately to USD and CNY', () => {
    expect(convertMicroUsd(1_500_000, 'USD', '7.25')).toBeCloseTo(1.5, 6);
    expect(convertMicroUsd(1_500_000, 'CNY', '7.25')).toBeCloseTo(10.875, 6);
    expect(formatCost(1_500_000, 'USD', '7.25')).toBe('$1.50');
    expect(formatCost(1_500_000, 'CNY', '7.25')).toBe('¥10.88');
  });

  it('formats percentages and reset countdowns cleanly', () => {
    expect(formatPercent(0.842, true)).toBe('84.2%');
    expect(formatCountdown(0)).toBe('已重置 / 空闲');
    expect(formatCountdown(3 * 3600 + 25 * 60)).toBe('3h 25m');
    expect(formatCountdown(2 * 86_400 + 4 * 3600)).toBe('2d 4h');
  });
});
