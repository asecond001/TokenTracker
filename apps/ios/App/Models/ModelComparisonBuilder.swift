import Foundation

public struct ModelComparisonRow: Equatable, Identifiable, Sendable {
    public let key: String
    public let modelId: String
    public let providerId: String
    public let requests: Int64
    public let totalTokens: Int64
    public let outputTokens: Int64
    public let costMicroUsd: Int64
    public let cacheSavedMicroUsd: Int64
    public let costPerMMicroUsd: Double?
    public let outputPricePerMMicroUsd: Double?
    public let cacheHitRatio: Double?
    public let meanTps: Double?
    public let p95Tps: Double?
    public let avgTtftMs: Double?
    public let avgDurationMs: Double?
    public let speedSamples: Int64

    public var id: String { key }
}

public struct ProviderComparisonRow: Equatable, Identifiable, Sendable {
    public let providerId: String
    public let modelCount: Int
    public let requests: Int64
    public let totalTokens: Int64
    public let costMicroUsd: Int64
    public let costShare: Double
    public let cacheSavedMicroUsd: Int64
    public let costPerMMicroUsd: Double?
    public let cacheHitRatio: Double?
    public let meanTps: Double?
    public let p95Tps: Double?
    public let avgTtftMs: Double?
    public let speedSamples: Int64

    public var id: String { providerId }
}

public enum ComparisonSortKey: String, CaseIterable, Identifiable, Sendable {
    case costMicroUsd = "cost_micro_usd"
    case totalTokens = "total_tokens"
    case costPerMMicroUsd = "cost_per_m_micro_usd"
    case meanTps = "mean_tps"
    case avgTtftMs = "avg_ttft_ms"
    case cacheHitRatio = "cache_hit_ratio"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .costMicroUsd: return L10n.tr("models.sort.cost", default: "花费")
        case .totalTokens: return L10n.tr("models.sort.tokens", default: "Tokens")
        case .costPerMMicroUsd: return L10n.tr("models.sort.cost_per_m", default: "均价/M")
        case .meanTps: return L10n.tr("models.sort.tps", default: "速率")
        case .avgTtftMs: return L10n.tr("models.sort.ttft", default: "首字延迟")
        case .cacheHitRatio: return L10n.tr("models.sort.cache", default: "缓存命中")
        }
    }
}

public protocol ComparisonSortableRow {
    func sortValue(for key: ComparisonSortKey) -> Double?
}

extension ModelComparisonRow: ComparisonSortableRow {
    public func sortValue(for key: ComparisonSortKey) -> Double? {
        switch key {
        case .costMicroUsd: return Double(costMicroUsd)
        case .totalTokens: return Double(totalTokens)
        case .costPerMMicroUsd: return costPerMMicroUsd
        case .meanTps: return meanTps
        case .avgTtftMs: return avgTtftMs
        case .cacheHitRatio: return cacheHitRatio
        }
    }
}

extension ProviderComparisonRow: ComparisonSortableRow {
    public func sortValue(for key: ComparisonSortKey) -> Double? {
        switch key {
        case .costMicroUsd: return Double(costMicroUsd)
        case .totalTokens: return Double(totalTokens)
        case .costPerMMicroUsd: return costPerMMicroUsd
        case .meanTps: return meanTps
        case .avgTtftMs: return avgTtftMs
        case .cacheHitRatio: return cacheHitRatio
        }
    }
}

public enum ModelComparisonBuilder {
    private static func costPerMillion(costMicro: Int64, tokens: Int64) -> Double? {
        tokens > 0 ? (Double(costMicro) / Double(tokens)) * 1_000_000.0 : nil
    }

    private static func cacheHitRatio(inputTokens: Int64, cacheReadTokens: Int64) -> Double? {
        let denom = inputTokens + cacheReadTokens
        return denom > 0 ? Double(cacheReadTokens) / Double(denom) : nil
    }

    private static func positiveOrNull(_ value: Double?) -> Double? {
        guard let value, value.isFinite, value > 0 else { return nil }
        return value
    }

    public static func buildModelComparison(
        models: ModelStatsReportDTO?,
        speed: SpeedLatencyReportDTO?
    ) -> [ModelComparisonRow] {
        var speedByKey: [String: ModelSpeedStatsDTO] = [:]
        for item in speed?.models ?? [] {
            speedByKey["\(item.providerId)::\(item.modelId)"] = item
        }

        var rowsByKey: [String: ModelComparisonRow] = [:]
        for m in models?.rows ?? [] {
            let key = "\(m.providerId)::\(m.modelId)"
            let s = speedByKey[key]
            rowsByKey[key] = ModelComparisonRow(
                key: key,
                modelId: m.modelId,
                providerId: m.providerId,
                requests: m.requests,
                totalTokens: m.totalTokens,
                outputTokens: m.outputTokens + m.reasoningTokens,
                costMicroUsd: m.estimatedCostMicroUsd,
                cacheSavedMicroUsd: m.cacheSavedMicroUsd,
                costPerMMicroUsd: costPerMillion(costMicro: m.estimatedCostMicroUsd, tokens: m.totalTokens),
                outputPricePerMMicroUsd: positiveOrNull(m.effectivePricing.map { Double($0.outputPerMMicroUsd) }),
                cacheHitRatio: cacheHitRatio(inputTokens: m.inputTokens, cacheReadTokens: m.cacheReadTokens),
                meanTps: positiveOrNull(s?.meanTps ?? (m.avgOutputTps > 0 ? m.avgOutputTps : nil)),
                p95Tps: positiveOrNull(s?.p95Tps),
                avgTtftMs: positiveOrNull(s?.avgTtftMs),
                avgDurationMs: positiveOrNull(s?.avgDurationMs),
                speedSamples: s?.sampleCount ?? 0
            )
        }

        for (key, s) in speedByKey where rowsByKey[key] == nil {
            rowsByKey[key] = ModelComparisonRow(
                key: key,
                modelId: s.modelId,
                providerId: s.providerId,
                requests: 0,
                totalTokens: 0,
                outputTokens: 0,
                costMicroUsd: 0,
                cacheSavedMicroUsd: 0,
                costPerMMicroUsd: nil,
                outputPricePerMMicroUsd: nil,
                cacheHitRatio: nil,
                meanTps: positiveOrNull(s.meanTps),
                p95Tps: positiveOrNull(s.p95Tps),
                avgTtftMs: positiveOrNull(s.avgTtftMs),
                avgDurationMs: positiveOrNull(s.avgDurationMs),
                speedSamples: s.sampleCount
            )
        }

        return rowsByKey.values.sorted { lhs, rhs in
            if lhs.costMicroUsd != rhs.costMicroUsd {
                return lhs.costMicroUsd > rhs.costMicroUsd
            }
            return lhs.totalTokens > rhs.totalTokens
        }
    }

    public static func buildProviderComparison(
        rows: [ModelComparisonRow],
        speed: SpeedLatencyReportDTO?,
        models: ModelStatsReportDTO?
    ) -> [ProviderComparisonRow] {
        var speedByProvider: [String: ProviderSpeedStatsDTO] = [:]
        for item in speed?.providers ?? [] {
            speedByProvider[item.providerId] = item
        }

        let totalCost = rows.reduce(Int64(0)) { $0 + $1.costMicroUsd }
        var cacheByProvider: [String: (input: Int64, cacheRead: Int64)] = [:]
        for m in models?.rows ?? [] {
            var current = cacheByProvider[m.providerId] ?? (0, 0)
            current.input += m.inputTokens
            current.cacheRead += m.cacheReadTokens
            cacheByProvider[m.providerId] = current
        }

        struct Accumulator {
            var modelCount = 0
            var requests: Int64 = 0
            var totalTokens: Int64 = 0
            var costMicroUsd: Int64 = 0
            var cacheSavedMicroUsd: Int64 = 0
        }

        var grouped: [String: Accumulator] = [:]
        for row in rows {
            var acc = grouped[row.providerId] ?? Accumulator()
            acc.modelCount += 1
            acc.requests += row.requests
            acc.totalTokens += row.totalTokens
            acc.costMicroUsd += row.costMicroUsd
            acc.cacheSavedMicroUsd += row.cacheSavedMicroUsd
            grouped[row.providerId] = acc
        }

        return grouped.map { providerId, acc in
            let s = speedByProvider[providerId]
            let c = cacheByProvider[providerId]
            let cacheRatio: Double? = if let c, c.input + c.cacheRead > 0 {
                Double(c.cacheRead) / Double(c.input + c.cacheRead)
            } else {
                nil
            }
            return ProviderComparisonRow(
                providerId: providerId,
                modelCount: acc.modelCount,
                requests: acc.requests,
                totalTokens: acc.totalTokens,
                costMicroUsd: acc.costMicroUsd,
                costShare: totalCost > 0 ? Double(acc.costMicroUsd) / Double(totalCost) : 0,
                cacheSavedMicroUsd: acc.cacheSavedMicroUsd,
                costPerMMicroUsd: costPerMillion(costMicro: acc.costMicroUsd, tokens: acc.totalTokens),
                cacheHitRatio: cacheRatio,
                meanTps: positiveOrNull(s?.meanTps),
                p95Tps: positiveOrNull(s?.p95Tps),
                avgTtftMs: positiveOrNull(s?.avgTtftMs),
                speedSamples: s?.sampleCount ?? 0
            )
        }
        .sorted { lhs, rhs in
            if lhs.costMicroUsd != rhs.costMicroUsd {
                return lhs.costMicroUsd > rhs.costMicroUsd
            }
            return lhs.totalTokens > rhs.totalTokens
        }
    }

    /// Sorts with `nil` values always placed last, regardless of sort direction.
    public static func sortComparisonRows<T: ComparisonSortableRow>(
        _ rows: [T],
        key: ComparisonSortKey,
        descending: Bool
    ) -> [T] {
        rows.sorted { lhs, rhs in
            switch (lhs.sortValue(for: key), rhs.sortValue(for: key)) {
            case (nil, nil): return false
            case (nil, _): return false
            case (_, nil): return true
            case let (av?, bv?):
                return descending ? (av > bv) : (av < bv)
            }
        }
    }
}
