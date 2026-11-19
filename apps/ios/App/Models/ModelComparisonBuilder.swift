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
    public let listCostMicroUsd: Int64
    public let subscriptionCostMicroUsd: Int64?
    public let outputPricePerMMicroUsd: Double?
    public let cacheHitRatio: Double?
    public let meanTps: Double?
    public let p95Tps: Double?
    public let avgTtftMs: Double?
    public let avgDurationMs: Double?
    public let speedSamples: Int64
    public let speedBasis: SpeedBasisDTO?

    public var id: String { key }

    public init(
        key: String,
        modelId: String,
        providerId: String,
        requests: Int64,
        totalTokens: Int64,
        outputTokens: Int64,
        costMicroUsd: Int64,
        cacheSavedMicroUsd: Int64,
        costPerMMicroUsd: Double?,
        listCostMicroUsd: Int64 = 0,
        subscriptionCostMicroUsd: Int64? = nil,
        outputPricePerMMicroUsd: Double?,
        cacheHitRatio: Double?,
        meanTps: Double?,
        p95Tps: Double?,
        avgTtftMs: Double?,
        avgDurationMs: Double?,
        speedSamples: Int64,
        speedBasis: SpeedBasisDTO? = nil
    ) {
        self.key = key
        self.modelId = modelId
        self.providerId = providerId
        self.requests = requests
        self.totalTokens = totalTokens
        self.outputTokens = outputTokens
        self.costMicroUsd = costMicroUsd
        self.cacheSavedMicroUsd = cacheSavedMicroUsd
        self.costPerMMicroUsd = costPerMMicroUsd
        self.listCostMicroUsd = listCostMicroUsd
        self.subscriptionCostMicroUsd = subscriptionCostMicroUsd
        self.outputPricePerMMicroUsd = outputPricePerMMicroUsd
        self.cacheHitRatio = cacheHitRatio
        self.meanTps = meanTps
        self.p95Tps = p95Tps
        self.avgTtftMs = avgTtftMs
        self.avgDurationMs = avgDurationMs
        self.speedSamples = speedSamples
        self.speedBasis = speedBasis
    }
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

public enum PriceBasis: String, CaseIterable, Identifiable, Sendable {
    case list
    case actual
    case subscription

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .list: return L10n.tr("models.frontier.basis_list", default: "API 标价")
        case .actual: return L10n.tr("models.frontier.basis_actual", default: "API 实付")
        case .subscription: return L10n.tr("models.frontier.basis_subscription", default: "订阅摊销")
        }
    }
}

public struct BasisPrice: Equatable, Sendable {
    public let perMMicroUsd: Double
    public let basis: PriceBasis

    public init(perMMicroUsd: Double, basis: PriceBasis) {
        self.perMMicroUsd = perMMicroUsd
        self.basis = basis
    }
}

public struct FrontierPointInput: Equatable, Sendable {
    public let key: String
    public let price: Double
    public let tps: Double

    public init(key: String, price: Double, tps: Double) {
        self.key = key
        self.price = price
        self.tps = tps
    }
}

public struct ParetoFrontierResult: Equatable, Sendable {
    public let frontier: [String]
    public let dominatedBy: [String: String]

    public init(frontier: [String], dominatedBy: [String: String]) {
        self.frontier = frontier
        self.dominatedBy = dominatedBy
    }
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
        case .costMicroUsd: return L10n.tr("models.sort.cost", default: "等效费用")
        case .totalTokens: return L10n.tr("models.sort.tokens", default: "Token")
        case .costPerMMicroUsd: return L10n.tr("models.sort.cost_per_m", default: "每百万 Token")
        case .meanTps: return L10n.tr("models.sort.tps", default: "平均速率")
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
    public static let minFrontierSpeedSamples: Int64 = 20

    private static func costPerMillion(costMicro: Int64, tokens: Int64) -> Double? {
        tokens > 0 ? (Double(costMicro) / Double(tokens)) * 1_000_000.0 : nil
    }

    private static func cacheHitRatio(
        inputTokens: Int64,
        cacheReadTokens: Int64,
        cacheWriteTokens: Int64
    ) -> Double? {
        let denom = inputTokens + cacheReadTokens + cacheWriteTokens
        return denom > 0 ? Double(cacheReadTokens) / Double(denom) : nil
    }

    private static func positiveOrNull(_ value: Double?) -> Double? {
        guard let value, value.isFinite, value > 0 else { return nil }
        return value
    }

    public static func isFrontierEligible(samples: Int64, speedBasis: SpeedBasisDTO?) -> Bool {
        samples >= minFrontierSpeedSamples && speedBasis != .endToEnd
    }

    public static func priceForBasis(row: ModelComparisonRow, basis: PriceBasis) -> BasisPrice? {
        guard row.totalTokens > 0 else { return nil }
        let cost: Int64
        let used: PriceBasis
        if basis == .list {
            cost = row.listCostMicroUsd
            used = .list
        } else if basis == .subscription, let subCost = row.subscriptionCostMicroUsd {
            cost = subCost
            used = .subscription
        } else {
            cost = row.costMicroUsd
            used = .actual
        }
        guard cost > 0 else { return nil }
        return BasisPrice(
            perMMicroUsd: (Double(cost) / Double(row.totalTokens)) * 1_000_000.0,
            basis: used
        )
    }

    /// Pareto frontier for "cheaper is better, faster is better": a point is
    /// dominated when another is no more expensive and no slower, and strictly
    /// better on at least one axis.
    public static func computeParetoFrontier(_ points: [FrontierPointInput]) -> ParetoFrontierResult {
        let validPoints = points.filter {
            $0.price.isFinite && $0.price > 0 && $0.tps.isFinite && $0.tps > 0
        }
        let sorted = validPoints.sorted { lhs, rhs in
            if lhs.price != rhs.price {
                return lhs.price < rhs.price
            }
            return lhs.tps > rhs.tps
        }
        var frontier: [FrontierPointInput] = []
        var bestTps = -Double.infinity
        for point in sorted {
            let last = frontier.last
            let tiesLast = last.map { $0.price == point.price && $0.tps == point.tps } ?? false
            if point.tps > bestTps || tiesLast {
                frontier.append(point)
                bestTps = point.tps
            }
        }
        let frontierKeys = Set(frontier.map(\.key))
        var dominatedBy: [String: String] = [:]
        for point in validPoints where !frontierKeys.contains(point.key) {
            if let dominator = frontier.first(where: { $0.price <= point.price && $0.tps >= point.tps }) {
                dominatedBy[point.key] = dominator.key
            }
        }
        return ParetoFrontierResult(
            frontier: frontier.map(\.key),
            dominatedBy: dominatedBy
        )
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
                listCostMicroUsd: m.listCostMicroUsd,
                subscriptionCostMicroUsd: m.subscriptionCostMicroUsd,
                outputPricePerMMicroUsd: positiveOrNull(m.effectivePricing.map { Double($0.outputPerMMicroUsd) }),
                cacheHitRatio: cacheHitRatio(
                    inputTokens: m.inputTokens,
                    cacheReadTokens: m.cacheReadTokens,
                    cacheWriteTokens: m.cacheWriteTokens
                ),
                meanTps: positiveOrNull(s?.meanTps ?? (m.avgOutputTps > 0 ? m.avgOutputTps : nil)),
                p95Tps: positiveOrNull(s?.p95Tps),
                avgTtftMs: positiveOrNull(s?.avgTtftMs),
                avgDurationMs: positiveOrNull(s?.avgDurationMs),
                speedSamples: s?.sampleCount ?? 0,
                speedBasis: s?.speedBasis
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
                listCostMicroUsd: 0,
                subscriptionCostMicroUsd: nil,
                outputPricePerMMicroUsd: nil,
                cacheHitRatio: nil,
                meanTps: positiveOrNull(s.meanTps),
                p95Tps: positiveOrNull(s.p95Tps),
                avgTtftMs: positiveOrNull(s.avgTtftMs),
                avgDurationMs: positiveOrNull(s.avgDurationMs),
                speedSamples: s.sampleCount,
                speedBasis: s.speedBasis
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
        var cacheByProvider: [String: (input: Int64, cacheRead: Int64, cacheWrite: Int64)] = [:]
        for m in models?.rows ?? [] {
            var current = cacheByProvider[m.providerId] ?? (0, 0, 0)
            current.input += m.inputTokens
            current.cacheRead += m.cacheReadTokens
            current.cacheWrite += m.cacheWriteTokens
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
            let cacheRatio = c.flatMap {
                cacheHitRatio(inputTokens: $0.input, cacheReadTokens: $0.cacheRead, cacheWriteTokens: $0.cacheWrite)
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
