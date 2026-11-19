import Charts
import Foundation
import SwiftUI

private enum ModelsSubTab: String, CaseIterable, Identifiable {
    case compare
    case cost
    case speed
    case catalog

    var id: String { rawValue }

    var title: String {
        switch self {
        case .compare: return L10n.tr("models.subtab.compare", default: "综合对比")
        case .cost: return L10n.tr("models.subtab.cost", default: "开销")
        case .speed: return L10n.tr("models.subtab.speed", default: "速度")
        case .catalog: return L10n.tr("models.subtab.catalog", default: "单价目录")
        }
    }
}

private struct SpeedBarPoint: Identifiable {
    let modelId: String
    let providerId: String
    let metric: String
    let value: Double

    var id: String { "\(providerId)::\(modelId)::\(metric)" }
}

private struct FrontierPlotPoint: Identifiable {
    let key: String
    let modelId: String
    let providerId: String
    let providerLabel: String
    let displayPrice: Double
    let xPlot: Double
    let meanTps: Double
    let tokenWeight: Double
    let samples: Int64
    let isFallback: Bool
    let speedBasis: SpeedBasisDTO?

    var id: String { key }
}

public struct ModelsCompareView: View {
    @Bindable var store: AppStore
    @State private var subTab: ModelsSubTab = .compare
    @State private var sortKey: ComparisonSortKey = .costMicroUsd
    @State private var sortDescending = true
    @State private var priceBasis: PriceBasis = .actual
    @State private var selectedFrontierKey: String?
    @State private var catalogQuery = ""
    @State private var editingPricingDraft: PricingOverrideDraft?

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                Picker(L10n.tr("models.subtab.picker", default: "子视图"), selection: $subTab) {
                    ForEach(ModelsSubTab.allCases) { tab in
                        Text(tab.title).tag(tab)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .accessibilityIdentifier("models.subTabPicker")

                HairlineDivider()

                switch subTab {
                case .compare:
                    compareSection
                case .cost:
                    costSection
                case .speed:
                    speedSection
                case .catalog:
                    catalogSection
                }
            }
            .padding(.bottom, 24)
        }
        .refreshable {
            await store.triggerManualSync()
        }
        .background(Color.clear)
        .sheet(item: $editingPricingDraft) { draft in
            ModelPricingOverrideSheet(draft: draft) { entry in
                await store.upsertModelPricing(entry)
            } onDelete: { pattern in
                await store.deleteModelPricingOverride(modelPattern: pattern)
            }
        }
    }

    // MARK: - 1. 综合对比

    private var compareSection: some View {
        let modelRows = ModelComparisonBuilder.buildModelComparison(
            models: store.modelsReport,
            speed: store.speedReport
        )
        let providerRows = ModelComparisonBuilder.buildProviderComparison(
            rows: modelRows,
            speed: store.speedReport,
            models: store.modelsReport
        )
        let sortedModels = ModelComparisonBuilder.sortComparisonRows(
            modelRows,
            key: sortKey,
            descending: sortDescending
        )
        let frontierPoints = buildFrontierPoints(from: modelRows)

        return VStack(alignment: .leading, spacing: 14) {
            if modelRows.isEmpty {
                emptyPlaceholder(L10n.tr("models.waiting", default: "等待模型数据"))
            } else {
                highlightsGrid(from: modelRows)

                if !frontierPoints.isEmpty {
                    HairlineDivider(insetLeading: 16)
                    paretoFrontierSection(points: frontierPoints)
                }

                if !providerRows.isEmpty {
                    HairlineDivider(insetLeading: 16)
                    providerRollupsSection(providerRows: providerRows)
                }

                HairlineDivider(insetLeading: 16)
                sortedComparisonTable(sortedModels: sortedModels)
            }
        }
        .padding(.top, 14)
    }

    @ViewBuilder
    private func highlightsGrid(from rows: [ModelComparisonRow]) -> some View {
        let topCost = rows.max { $0.costMicroUsd < $1.costMicroUsd }
        let fastest = rows.filter { $0.meanTps != nil }.max { ($0.meanTps ?? 0) < ($1.meanTps ?? 0) }
        let bestCache = rows.filter { $0.cacheHitRatio != nil }.max { ($0.cacheHitRatio ?? 0) < ($1.cacheHitRatio ?? 0) }
        let cheapest = rows.filter { ($0.costPerMMicroUsd ?? 0) > 0 }.min { ($0.costPerMMicroUsd ?? .infinity) < ($1.costPerMMicroUsd ?? .infinity) }

        LazyVGrid(
            columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)],
            spacing: 10
        ) {
            highlightCard(
                title: L10n.tr("models.highlight.top_cost", default: "消费最高"),
                model: topCost?.modelId ?? "—",
                metric: topCost.map {
                    TokenVaneFormatters.formatCurrency($0.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate)
                } ?? "—"
            )
            highlightCard(
                title: L10n.tr("models.highlight.fastest", default: "最快输出"),
                model: fastest?.modelId ?? "—",
                metric: TokenVaneFormatters.formatTps(fastest?.meanTps)
            )
            highlightCard(
                title: L10n.tr("models.highlight.best_cache", default: "最佳缓存"),
                model: bestCache?.modelId ?? "—",
                metric: bestCache?.cacheHitRatio.map { TokenVaneFormatters.formatPercent($0 * 100.0) } ?? "—"
            )
            highlightCard(
                title: L10n.tr("models.highlight.cheapest", default: "最省单价"),
                model: cheapest?.modelId ?? "—",
                metric: cheapest?.costPerMMicroUsd.map {
                    "\(TokenVaneFormatters.formatCurrency(Int64($0.rounded()), currency: store.currency, rateString: store.usdToCnyRate))/M"
                } ?? "—"
            )
        }
        .padding(.horizontal, 16)
    }

    private func highlightCard(title: String, model: String, metric: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text(metric)
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
            Text(model)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private func buildFrontierPoints(from rows: [ModelComparisonRow]) -> [FrontierPlotPoint] {
        rows.compactMap { row -> FrontierPlotPoint? in
            guard let basisPrice = ModelComparisonBuilder.priceForBasis(row: row, basis: priceBasis),
                  let tps = row.meanTps,
                  tps > 0 else {
                return nil
            }
            let roundedMicro = Int64(basisPrice.perMMicroUsd.rounded())
            let displayVal = TokenVaneFormatters.convertMicroUsd(
                max(Int64(1), roundedMicro),
                currency: store.currency,
                rateString: store.usdToCnyRate
            )
            guard displayVal.isFinite, displayVal > 0 else { return nil }
            let clamped = max(displayVal, 0.0001)
            return FrontierPlotPoint(
                key: row.key,
                modelId: row.modelId,
                providerId: row.providerId,
                providerLabel: TokenVaneFormatters.providerDisplayName(row.providerId),
                displayPrice: displayVal,
                xPlot: -Foundation.log10(clamped),
                meanTps: tps,
                tokenWeight: max(1.0, Double(row.totalTokens)),
                samples: row.speedSamples,
                isFallback: basisPrice.basis != priceBasis,
                speedBasis: row.speedBasis
            )
        }
    }

    private func paretoFrontierSection(points: [FrontierPlotPoint]) -> some View {
        let unitSymbol = store.currency.uppercased() == "CNY" ? "¥" : "$"
        let eligible = points.filter {
            ModelComparisonBuilder.isFrontierEligible(samples: $0.samples, speedBasis: $0.speedBasis)
        }
        let frontierResult = ModelComparisonBuilder.computeParetoFrontier(
            eligible.map { FrontierPointInput(key: $0.key, price: $0.displayPrice, tps: $0.meanTps) }
        )
        let frontierSet = Set(frontierResult.frontier)
        let byKey = Dictionary(uniqueKeysWithValues: points.map { ($0.key, $0) })
        let frontierLinePoints = frontierResult.frontier.compactMap { byKey[$0] }
        let maxTokens = max(1.0, points.map(\.tokenWeight).max() ?? 1.0)

        let rawMinX = points.map(\.xPlot).min() ?? -1.0
        let rawMaxX = points.map(\.xPlot).max() ?? 1.0
        let spanX = rawMaxX - rawMinX
        let padX = spanX < 0.05 ? 0.35 : max(0.08, spanX * 0.14)
        let domainX: ClosedRange<Double> = (rawMinX - padX)...(rawMaxX + padX)
        let tickXValues = buildLogPriceTicks(domainX: domainX)

        let minEligiblePrice = eligible.map(\.displayPrice).min() ?? 0.0
        let maxEligibleTps = eligible.map(\.meanTps).max() ?? 0.0
        let medianEligiblePrice = medianValue(eligible.map(\.displayPrice))
        let medianEligibleTps = medianValue(eligible.map(\.meanTps))
        let showBetterArea = eligible.count >= 3 && minEligiblePrice > 0 && medianEligiblePrice > 0

        let activeKey = (selectedFrontierKey.flatMap { byKey[$0]?.key })
            ?? frontierResult.frontier.first
            ?? points.first?.key
        let activePoint = activeKey.flatMap { byKey[$0] }

        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(L10n.tr("models.chart.pareto_title", default: "性价比前沿"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text(L10n.tr("models.chart.better_corner", default: "更便宜 · 更快 ↗"))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            Picker(L10n.tr("models.frontier.basis_picker", default: "价格口径"), selection: $priceBasis) {
                ForEach(PriceBasis.allCases) { basis in
                    Text(basis.label).tag(basis)
                }
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("models.priceBasisPicker")

            Chart {
                if showBetterArea {
                    RectangleMark(
                        xStart: .value("Price", -Foundation.log10(max(medianEligiblePrice, 0.0001))),
                        xEnd: .value("Price", -Foundation.log10(max(minEligiblePrice * 0.92, 0.0001))),
                        yStart: .value("TPS", medianEligibleTps),
                        yEnd: .value("TPS", maxEligibleTps * 1.04)
                    )
                    .foregroundStyle(SemanticColors.accentIndigo.opacity(0.08))
                }

                frontierLineMarks(frontierLinePoints)

                ForEach(points) { pt in
                    let onFrontier = frontierSet.contains(pt.key)
                    let eligiblePoint = ModelComparisonBuilder.isFrontierEligible(
                        samples: pt.samples,
                        speedBasis: pt.speedBasis
                    )
                    let opacity: Double = onFrontier ? 0.95 : (eligiblePoint ? 0.30 : 0.62)
                    let symbolArea = CGFloat(52.0 + Foundation.sqrt(pt.tokenWeight / maxTokens) * 190.0)
                    let showLabel = onFrontier && frontierLinePoints.count <= 6
                    scatterPointMark(
                        pt,
                        size: symbolArea,
                        color: TokenVaneFormatters.providerColor(pt.providerId).opacity(opacity),
                        showLabel: showLabel
                    )
                }
            }
            .chartXScale(domain: domainX)
            .chartYScale(domain: .automatic(includesZero: false))
            .chartYAxis {
                AxisMarks(position: .trailing) { value in
                    AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5))
                        .foregroundStyle(SemanticColors.hairline)
                    AxisValueLabel {
                        if let v = value.as(Double.self) {
                            Text(String(format: "%.0f", locale: Locale(identifier: "en_US_POSIX"), v))
                                .font(.caption2)
                                .tabularMetrics()
                        }
                    }
                }
            }
            .chartXAxis {
                AxisMarks(values: tickXValues) { value in
                    AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5, dash: [3, 3]))
                        .foregroundStyle(SemanticColors.hairline)
                    AxisValueLabel {
                        if let xVal = value.as(Double.self) {
                            let price = Foundation.pow(10.0, -xVal)
                            Text(formatAxisPrice(price, symbol: unitSymbol))
                                .font(.caption2)
                                .tabularMetrics()
                        }
                    }
                }
            }
            .frame(height: 210)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(points) { pt in
                        let selected = pt.key == activePoint?.key
                        let onFrontier = frontierSet.contains(pt.key)
                        Button {
                            selectedFrontierKey = pt.key
                        } label: {
                            HStack(spacing: 5) {
                                Circle()
                                    .fill(TokenVaneFormatters.providerColor(pt.providerId))
                                    .frame(width: 6, height: 6)
                                Text(pt.modelId)
                                    .font(.caption2.weight(selected ? .semibold : .regular))
                                    .lineLimit(1)
                                if onFrontier {
                                    Image(systemName: "star.fill")
                                        .font(.system(size: 8))
                                        .foregroundStyle(SemanticColors.accentIndigo)
                                }
                            }
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .background(
                                selected ? SemanticColors.accentIndigo.opacity(0.14) : SemanticColors.surfaceSubtle,
                                in: Capsule()
                            )
                            .overlay(
                                Capsule()
                                    .strokeBorder(
                                        selected ? SemanticColors.accentIndigo.opacity(0.6) : SemanticColors.hairline,
                                        lineWidth: 0.8
                                    )
                            )
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            if let pt = activePoint {
                frontierDetailCard(
                    point: pt,
                    unitSymbol: unitSymbol,
                    frontierSet: frontierSet,
                    dominatedBy: frontierResult.dominatedBy,
                    byKey: byKey
                )
            }

            Text(L10n.tr("models.frontier.legend_note", default: "★ 性价比前沿 · 淡点已被超越 · ◇ 无订阅回退实付 · □ 端到端口径单列"))
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 16)
    }

    @ChartContentBuilder
    private func frontierLineMarks(_ frontierPoints: [FrontierPlotPoint]) -> some ChartContent {
        ForEach(frontierPoints) { pt in
            LineMark(
                x: .value("Price", pt.xPlot),
                y: .value("TPS", pt.meanTps),
                series: .value("Series", "frontier")
            )
            .interpolationMethod(.linear)
            .lineStyle(StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
            .foregroundStyle(Color.secondary.opacity(0.75))
        }
    }

    @ChartContentBuilder
    private func scatterPointMark(
        _ pt: FrontierPlotPoint,
        size: CGFloat,
        color: Color,
        showLabel: Bool
    ) -> some ChartContent {
        if pt.isFallback {
            PointMark(x: .value("Price", pt.xPlot), y: .value("TPS", pt.meanTps))
                .symbol(.diamond)
                .symbolSize(size)
                .foregroundStyle(color)
                .annotation(position: .top, spacing: 2) {
                    if showLabel {
                        frontierLabelView(pt.modelId)
                    }
                }
        } else if pt.speedBasis == .endToEnd {
            PointMark(x: .value("Price", pt.xPlot), y: .value("TPS", pt.meanTps))
                .symbol(.square)
                .symbolSize(size)
                .foregroundStyle(color)
                .annotation(position: .top, spacing: 2) {
                    if showLabel {
                        frontierLabelView(pt.modelId)
                    }
                }
        } else {
            PointMark(x: .value("Price", pt.xPlot), y: .value("TPS", pt.meanTps))
                .symbol(.circle)
                .symbolSize(size)
                .foregroundStyle(color)
                .annotation(position: .top, spacing: 2) {
                    if showLabel {
                        frontierLabelView(pt.modelId)
                    }
                }
        }
    }

    private func frontierLabelView(_ modelId: String) -> some View {
        Text(modelId)
            .font(.system(size: 9, weight: .medium))
            .foregroundStyle(.secondary)
            .lineLimit(1)
    }

    private func frontierDetailCard(
        point pt: FrontierPlotPoint,
        unitSymbol: String,
        frontierSet: Set<String>,
        dominatedBy: [String: String],
        byKey: [String: FrontierPlotPoint]
    ) -> some View {
        let basisText = pt.isFallback ? PriceBasis.actual.label : priceBasis.label
        let priceStr = formatAxisPrice(pt.displayPrice, symbol: unitSymbol)
        let status = classifyFrontierStatus(
            point: pt,
            frontierSet: frontierSet,
            dominatedBy: dominatedBy,
            byKey: byKey
        )

        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Circle()
                    .fill(TokenVaneFormatters.providerColor(pt.providerId))
                    .frame(width: 7, height: 7)
                Text(pt.modelId)
                    .font(.caption.weight(.semibold))
                    .lineLimit(1)
                Text("· \(pt.providerLabel)")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                Spacer()
                Text(status.badgeText)
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(status.accentColor)
            }
            HStack(spacing: 10) {
                Text("\(basisText) \(priceStr)/M")
                Text("·")
                Text("\(TokenVaneFormatters.formatTps(pt.meanTps))")
                Spacer()
                Text(L10n.trFormat("models.speed.samples_count", default: "%lld 样本", pt.samples))
                    .foregroundStyle(.secondary)
            }
            .font(.caption2)
            .tabularMetrics()
        }
        .padding(10)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private func classifyFrontierStatus(
        point pt: FrontierPlotPoint,
        frontierSet: Set<String>,
        dominatedBy: [String: String],
        byKey: [String: FrontierPlotPoint]
    ) -> (badgeText: String, accentColor: Color) {
        if pt.speedBasis == .endToEnd {
            return (
                L10n.tr("models.frontier.status_end_to_end", default: "端到端口径 · 含首字延迟，单独展示"),
                .secondary
            )
        }
        if pt.samples < ModelComparisonBuilder.minFrontierSpeedSamples {
            return (
                L10n.trFormat(
                    "models.frontier.status_accumulating",
                    default: "速度样本积累中（%lld/%lld）",
                    pt.samples,
                    ModelComparisonBuilder.minFrontierSpeedSamples
                ),
                SemanticColors.statusWarning
            )
        }
        if frontierSet.contains(pt.key) {
            return (
                L10n.tr("models.frontier.status_on_frontier", default: "性价比前沿"),
                SemanticColors.statusLive
            )
        }
        if let domKey = dominatedBy[pt.key], let dom = byKey[domKey] {
            return (
                L10n.trFormat("models.frontier.status_dominated", default: "被 %@ 全面超越", dom.modelId),
                .secondary
            )
        }
        return ("—", .secondary)
    }

    private func medianValue(_ values: [Double]) -> Double {
        let sorted = values.sorted()
        guard !sorted.isEmpty else { return 0 }
        let mid = sorted.count / 2
        if sorted.count % 2 == 1 {
            return sorted[mid]
        }
        return (sorted[mid - 1] + sorted[mid]) / 2.0
    }

    private func buildLogPriceTicks(domainX: ClosedRange<Double>) -> [Double] {
        let minDomainPrice = Foundation.pow(10.0, -domainX.upperBound)
        let maxDomainPrice = Foundation.pow(10.0, -domainX.lowerBound)
        var candidates: [Double] = []
        for exp in -4...4 {
            let base = Foundation.pow(10.0, Double(exp))
            for mult in [1.0, 2.0, 5.0] {
                let tick = base * mult
                if tick >= minDomainPrice && tick <= maxDomainPrice {
                    candidates.append(tick)
                }
            }
        }
        if candidates.count >= 2 {
            let step = max(1, candidates.count / 4)
            var reduced: [Double] = []
            for idx in stride(from: 0, to: candidates.count, by: step) {
                reduced.append(candidates[idx])
            }
            return reduced.map { -Foundation.log10($0) }.sorted()
        }
        let lower = domainX.lowerBound
        let upper = domainX.upperBound
        let mid = (lower + upper) / 2.0
        return [lower, mid, upper]
    }

    private func formatAxisPrice(_ value: Double, symbol: String) -> String {
        let posix = Locale(identifier: "en_US_POSIX")
        if value >= 100.0 {
            return String(format: "%@%.0f", locale: posix, symbol, value)
        }
        if value >= 10.0 {
            return String(format: "%@%.1f", locale: posix, symbol, value)
        }
        if value >= 0.1 {
            return String(format: "%@%.2f", locale: posix, symbol, value)
        }
        return String(format: "%@%.3f", locale: posix, symbol, value)
    }

    private func providerRollupsSection(providerRows: [ProviderComparisonRow]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L10n.tr("models.providers.title", default: "服务商综合表现"))
                .font(.subheadline.weight(.semibold))
            ForEach(providerRows) { p in
                HStack(spacing: 8) {
                    Circle()
                        .fill(TokenVaneFormatters.providerColor(p.providerId))
                        .frame(width: 7, height: 7)
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(TokenVaneFormatters.providerDisplayName(p.providerId))
                                .font(.caption.weight(.medium))
                            Text(L10n.trFormat("models.providers.count", default: "%lld 模型", Int64(p.modelCount)))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        Text("\(TokenVaneFormatters.formatTps(p.meanTps)) · 缓存 \(p.cacheHitRatio.map { TokenVaneFormatters.formatPercent($0 * 100.0, decimals: 0) } ?? "—") · TTFT \(p.avgTtftMs.map { TokenVaneFormatters.formatDurationMs($0) } ?? "—")")
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 2) {
                        Text(TokenVaneFormatters.formatCurrency(p.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                            .font(.caption.weight(.semibold))
                            .tabularMetrics()
                        Text(TokenVaneFormatters.formatPercent(p.costShare * 100.0))
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 3)
                HairlineDivider()
            }
        }
        .padding(.horizontal, 16)
    }

    private func sortedComparisonTable(sortedModels: [ModelComparisonRow]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(L10n.tr("models.table.title", default: "模型横向对比"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Menu {
                    ForEach(ComparisonSortKey.allCases) { key in
                        Button {
                            if sortKey == key {
                                sortDescending.toggle()
                            } else {
                                sortKey = key
                                sortDescending = (key != .avgTtftMs)
                            }
                        } label: {
                            if sortKey == key {
                                Label(key.label, systemImage: sortDescending ? "arrow.down" : "arrow.up")
                            } else {
                                Text(key.label)
                            }
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Text(sortKey.label)
                            .font(.caption2.weight(.medium))
                        Image(systemName: sortDescending ? "arrow.down" : "arrow.up")
                            .font(.caption2)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                }
            }

            ForEach(sortedModels) { row in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(row.modelId)
                            .font(.subheadline.weight(.medium))
                            .lineLimit(1)
                        Text(TokenVaneFormatters.providerShortCode(row.providerId))
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(TokenVaneFormatters.providerColor(row.providerId))
                        Spacer()
                        Text(TokenVaneFormatters.formatCurrency(row.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                            .font(.subheadline.weight(.semibold))
                            .tabularMetrics()
                    }
                    HStack(spacing: 10) {
                        Text("Token \(TokenVaneFormatters.formatCompactTokens(row.totalTokens))")
                        if let perM = row.costPerMMicroUsd {
                            Text("\(TokenVaneFormatters.formatCurrency(Int64(perM.rounded()), currency: store.currency, rateString: store.usdToCnyRate))/M")
                        }
                        Spacer()
                        Text("\(TokenVaneFormatters.formatTps(row.meanTps)) · \(row.cacheHitRatio.map { TokenVaneFormatters.formatPercent($0 * 100.0, decimals: 0) } ?? "—")")
                    }
                    .font(.caption2)
                    .tabularMetrics()
                    .foregroundStyle(.secondary)
                }
                .padding(.vertical, 4)
                HairlineDivider()
            }
        }
        .padding(.horizontal, 16)
    }

    // MARK: - 2. 开销明细 + 改价

    private var costSection: some View {
        let report = store.modelsReport
        let rows = report?.rows ?? []
        let allocations = report?.subscriptionAllocations ?? []

        return VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(L10n.tr("models.cost.total", default: "周期等效花费"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(TokenVaneFormatters.formatCurrency(report?.totalCostMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate))
                        .font(.title3.weight(.semibold))
                        .tabularMetrics()
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    Text(L10n.tr("models.cost.saved", default: "缓存节省"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(TokenVaneFormatters.formatCurrency(report?.totalCacheSavedMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate))
                        .font(.title3.weight(.semibold))
                        .tabularMetrics()
                        .foregroundStyle(SemanticColors.statusLive)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)

            if !allocations.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.tr("models.cost.subscription_allocations", default: "订阅摊销"))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(allocations) { alloc in
                        HStack(spacing: 8) {
                            Circle()
                                .fill(TokenVaneFormatters.providerColor(alloc.providerId))
                                .frame(width: 6, height: 6)
                            Text(TokenVaneFormatters.providerDisplayName(alloc.providerId))
                                .font(.caption2.weight(.medium))
                            Text(alloc.planNames.joined(separator: " + "))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                            Spacer()
                            Text(TokenVaneFormatters.formatCurrency(alloc.periodFeeMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.caption2.weight(.semibold))
                                .tabularMetrics()
                        }
                    }
                }
                .padding(10)
                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .padding(.horizontal, 16)
            }

            HairlineDivider(insetLeading: 16)

            if rows.isEmpty {
                emptyPlaceholder(L10n.tr("models.waiting", default: "等待模型数据"))
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(rows) { row in
                        VStack(alignment: .leading, spacing: 5) {
                            HStack(spacing: 6) {
                                Circle()
                                    .fill(TokenVaneFormatters.providerColor(row.providerId))
                                    .frame(width: 7, height: 7)
                                Text(row.modelId)
                                    .font(.subheadline.weight(.medium))
                                    .lineLimit(1)
                                if row.effectivePricing?.isUserOverride == true {
                                    Text(L10n.tr("models.pricing.override_badge", default: "自定义价"))
                                        .font(.caption2)
                                        .foregroundStyle(SemanticColors.accentIndigo)
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1)
                                        .background(SemanticColors.accentIndigo.opacity(0.12), in: RoundedRectangle(cornerRadius: 3, style: .continuous))
                                }
                                Spacer()
                                Button(L10n.tr("models.action.edit_price", default: "改价")) {
                                    editingPricingDraft = PricingOverrideDraft.from(row: row)
                                }
                                .font(.caption2.weight(.medium))
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                            }

                            HStack {
                                Text("In \(TokenVaneFormatters.formatCompactTokens(row.inputTokens)) · Out \(TokenVaneFormatters.formatCompactTokens(row.outputTokens)) · Cache \(TokenVaneFormatters.formatCompactTokens(row.cacheReadTokens))")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                                Spacer()
                                Text(TokenVaneFormatters.formatCurrency(row.estimatedCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                    .font(.caption.weight(.semibold))
                                    .tabularMetrics()
                            }

                            HStack {
                                Text("\(row.requests) req · 命中 \(TokenVaneFormatters.formatPercent(row.cacheHitPercent, decimals: 0))")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                                Spacer()
                                if let subCost = row.subscriptionCostMicroUsd, subCost > 0 {
                                    Text(
                                        L10n.trFormat(
                                            "models.cost.sub_share_value",
                                            default: "摊销 %@",
                                            TokenVaneFormatters.formatCurrency(subCost, currency: store.currency, rateString: store.usdToCnyRate)
                                        )
                                    )
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                                }
                                Text("省 \(TokenVaneFormatters.formatCurrency(row.cacheSavedMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(SemanticColors.statusLive)
                            }
                        }
                        .padding(.vertical, 4)
                        HairlineDivider()
                    }
                }
                .padding(.horizontal, 16)
            }
        }
    }

    // MARK: - 3. 速度分析

    private var speedSection: some View {
        let report = store.speedReport
        let models = report?.models ?? []
        let fromRollups = report?.fromHourlyRollups ?? false
        let barPoints = buildSpeedBarPoints(from: Array(models.prefix(8)))

        return VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(L10n.tr("models.speed.chart_title", default: "均值 vs P95 速率 (tok/s)"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                if fromRollups {
                    Text(L10n.tr("models.rollups_badge", default: "小时聚合"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .overlay(
                            RoundedRectangle(cornerRadius: 3, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)

            if models.isEmpty {
                emptyPlaceholder(L10n.tr("models.speed.empty", default: "暂无足够有效流式生成样本"))
            } else {
                Chart(barPoints) { pt in
                    BarMark(
                        x: .value(L10n.tr("models.sort.tps", default: "速率"), pt.value),
                        y: .value(L10n.tr("analytics.dimension.model", default: "模型"), pt.modelId)
                    )
                    .foregroundStyle(by: .value("Stat", pt.metric))
                    .position(by: .value("Stat", pt.metric))
                }
                .chartForegroundStyleScale([
                    L10n.tr("models.speed.mean", default: "均值"): SemanticColors.accentIndigo,
                    "P95": SemanticColors.statusLive
                ])
                .chartLegend(position: .bottom, alignment: .leading)
                .frame(height: CGFloat(max(140, min(320, models.prefix(8).count * 38))))
                .padding(.horizontal, 16)

                HairlineDivider(insetLeading: 16)

                VStack(alignment: .leading, spacing: 8) {
                    ForEach(models) { item in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Circle()
                                    .fill(TokenVaneFormatters.providerColor(item.providerId))
                                    .frame(width: 7, height: 7)
                                Text(item.modelId)
                                    .font(.subheadline.weight(.medium))
                                    .lineLimit(1)
                                if let basis = item.speedBasis {
                                    Text(
                                        basis == .decode
                                            ? L10n.tr("models.speed.basis_decode", default: "解码窗")
                                            : L10n.tr("models.speed.basis_end_to_end", default: "端到端")
                                    )
                                    .font(.caption2)
                                    .foregroundStyle(basis == .decode ? SemanticColors.accentIndigo : .secondary)
                                    .padding(.horizontal, 5)
                                    .padding(.vertical, 1)
                                    .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 3, style: .continuous))
                                }
                                Spacer()
                                Text(TokenVaneFormatters.formatTps(item.meanTps))
                                    .font(.subheadline.weight(.semibold))
                                    .tabularMetrics()
                            }
                            HStack(spacing: 10) {
                                Text("P50 \(String(format: "%.1f", item.p50Tps))")
                                Text("P95 \(String(format: "%.1f", item.p95Tps))")
                                Text("Max \(String(format: "%.1f", item.maxTps))")
                                Spacer()
                                if let ttft = item.avgTtftMs {
                                    Text("TTFT \(TokenVaneFormatters.formatDurationMs(ttft))")
                                }
                                Text("\(item.sampleCount) 样本")
                            }
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                        }
                        .padding(.vertical, 4)
                        HairlineDivider()
                    }
                }
                .padding(.horizontal, 16)
            }
        }
    }

    private func buildSpeedBarPoints(from models: [ModelSpeedStatsDTO]) -> [SpeedBarPoint] {
        let meanLabel = L10n.tr("models.speed.mean", default: "均值")
        var out: [SpeedBarPoint] = []
        for m in models {
            out.append(SpeedBarPoint(modelId: m.modelId, providerId: m.providerId, metric: meanLabel, value: m.meanTps))
            if m.p95Tps > 0 {
                out.append(SpeedBarPoint(modelId: m.modelId, providerId: m.providerId, metric: "P95", value: m.p95Tps))
            }
        }
        return out
    }

    // MARK: - 4. 单价目录

    private var catalogSection: some View {
        let allEntries = store.modelsReport?.catalog ?? []
        let query = catalogQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let filtered = allEntries
            .filter { entry in
                if query.isEmpty { return true }
                return entry.modelPattern.lowercased().contains(query)
                    || entry.providerId.lowercased().contains(query)
            }
            .sorted { lhs, rhs in
                if lhs.isUserOverride != rhs.isUserOverride {
                    return lhs.isUserOverride
                }
                return lhs.modelPattern < rhs.modelPattern
            }

        return VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    TextField(L10n.tr("models.catalog.search", default: "搜索模型名或服务商"), text: $catalogQuery)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.subheadline)
                        .padding(8)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )

                    Button {
                        Task { _ = await store.syncAllCatalogsNow() }
                    } label: {
                        if store.isSyncingCatalogs {
                            ProgressView()
                                .controlSize(.small)
                                .padding(.horizontal, 8)
                        } else {
                            Label(L10n.tr("models.catalog.sync", default: "同步"), systemImage: "arrow.clockwise")
                                .font(.caption.weight(.medium))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
                                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                        }
                    }
                    .disabled(store.isSyncingCatalogs)
                }

                HStack {
                    Text(
                        L10n.trFormat(
                            "models.catalog.meta",
                            default: "共 %lld 条 · 更新 %@",
                            Int64(allEntries.count),
                            TokenVaneFormatters.formatRelativeSync(atMs: store.bootstrap?.preferences.lastPricingSyncAtMs)
                        )
                    )
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    Spacer()
                    Button(L10n.tr("models.catalog.add_override", default: "+ 自定义单价")) {
                        editingPricingDraft = PricingOverrideDraft.empty(defaultProvider: store.selectedProviderId ?? "claude_code")
                    }
                    .font(.caption.weight(.medium))
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)

            HairlineDivider(insetLeading: 16)

            if filtered.isEmpty {
                emptyPlaceholder(L10n.tr("models.catalog.empty", default: "暂无匹配单价条目"))
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(filtered.prefix(150)) { entry in
                        HStack(alignment: .top, spacing: 8) {
                            VStack(alignment: .leading, spacing: 3) {
                                HStack(spacing: 6) {
                                    Text(entry.modelPattern)
                                        .font(.caption.weight(.semibold))
                                        .lineLimit(1)
                                    Text(TokenVaneFormatters.providerShortCode(entry.providerId))
                                        .font(.caption2.weight(.semibold))
                                        .foregroundStyle(TokenVaneFormatters.providerColor(entry.providerId))
                                    if entry.isUserOverride {
                                        Text(L10n.tr("models.pricing.override_badge", default: "自定义"))
                                            .font(.caption2)
                                            .foregroundStyle(SemanticColors.accentIndigo)
                                    }
                                }
                                Text("In \(formatUsdPerM(entry.inputPerMMicroUsd)) · Out \(formatUsdPerM(entry.outputPerMMicroUsd)) · Cache \(formatUsdPerM(entry.cacheReadPerMMicroUsd))")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            HStack(spacing: 8) {
                                if entry.isUserOverride {
                                    Button(L10n.tr("models.pricing.revert", default: "还原")) {
                                        Task { _ = await store.deleteModelPricingOverride(modelPattern: entry.modelPattern) }
                                    }
                                    .font(.caption2)
                                    .foregroundStyle(SemanticColors.statusDanger)
                                }
                                Button(L10n.tr("models.action.edit", default: "编辑")) {
                                    editingPricingDraft = PricingOverrideDraft.from(entry: entry)
                                }
                                .font(.caption2.weight(.medium))
                            }
                        }
                        .padding(.vertical, 3)
                        HairlineDivider()
                    }
                }
                .padding(.horizontal, 16)
            }
        }
    }

    private func formatUsdPerM(_ microUsd: Int64) -> String {
        let val = Double(microUsd) / 1_000_000.0
        if val <= 0 { return "$0" }
        return String(format: "$%.2f", locale: Locale(identifier: "en_US_POSIX"), val)
    }

    private func emptyPlaceholder(_ message: String) -> some View {
        Text(message)
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 16)
            .padding(.vertical, 28)
    }
}

private struct PricingOverrideDraft: Identifiable {
    let id = UUID()
    var isEditingExistingOverride: Bool
    var modelPattern: String
    var providerId: String
    var inputUsd: String
    var outputUsd: String
    var cacheReadUsd: String
    var cacheWriteUsd: String
    var reasoningUsd: String

    static func empty(defaultProvider: String) -> PricingOverrideDraft {
        PricingOverrideDraft(
            isEditingExistingOverride: false,
            modelPattern: "",
            providerId: defaultProvider,
            inputUsd: "3.0",
            outputUsd: "15.0",
            cacheReadUsd: "0.3",
            cacheWriteUsd: "3.75",
            reasoningUsd: "15.0"
        )
    }

    static func from(entry: ModelPricingEntryDTO) -> PricingOverrideDraft {
        PricingOverrideDraft(
            isEditingExistingOverride: entry.isUserOverride,
            modelPattern: entry.modelPattern,
            providerId: entry.providerId,
            inputUsd: formatRate(entry.inputPerMMicroUsd),
            outputUsd: formatRate(entry.outputPerMMicroUsd),
            cacheReadUsd: formatRate(entry.cacheReadPerMMicroUsd),
            cacheWriteUsd: formatRate(entry.cacheWritePerMMicroUsd),
            reasoningUsd: formatRate(entry.reasoningPerMMicroUsd)
        )
    }

    static func from(row: ModelStatRowDTO) -> PricingOverrideDraft {
        if let eff = row.effectivePricing {
            return from(entry: eff)
        }
        return PricingOverrideDraft(
            isEditingExistingOverride: false,
            modelPattern: row.modelId,
            providerId: row.providerId,
            inputUsd: "0",
            outputUsd: "0",
            cacheReadUsd: "0",
            cacheWriteUsd: "0",
            reasoningUsd: "0"
        )
    }

    private static func formatRate(_ micro: Int64) -> String {
        String(format: "%.4g", locale: Locale(identifier: "en_US_POSIX"), Double(micro) / 1_000_000.0)
    }
}

private struct ModelPricingOverrideSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State var draft: PricingOverrideDraft
    let onSave: @MainActor (ModelPricingEntryDTO) async -> Bool
    let onDelete: @MainActor (String) async -> Bool

    var body: some View {
        NavigationStack {
            Form {
                Section(L10n.tr("models.override.target", default: "模型匹配规则")) {
                    TextField(L10n.tr("models.override.pattern_placeholder", default: "模型名或前缀 (如 claude-sonnet-4-5)"), text: $draft.modelPattern)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField(L10n.tr("models.override.provider", default: "服务商 ID"), text: $draft.providerId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }

                Section(L10n.tr("models.override.rates_usd_per_m", default: "USD / 1M Tokens 单价")) {
                    rateField("Input", text: $draft.inputUsd)
                    rateField("Output", text: $draft.outputUsd)
                    rateField("Cache Read", text: $draft.cacheReadUsd)
                    rateField("Cache Write", text: $draft.cacheWriteUsd)
                    rateField("Reasoning", text: $draft.reasoningUsd)
                }

                if draft.isEditingExistingOverride {
                    Section {
                        Button(role: .destructive) {
                            let pattern = draft.modelPattern
                            Task {
                                if await onDelete(pattern) {
                                    dismiss()
                                }
                            }
                        } label: {
                            Text(L10n.tr("models.override.delete", default: "恢复目录默认价格"))
                        }
                    }
                }
            }
            .navigationTitle(L10n.tr("models.override.title", default: "自定义模型单价"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.tr("pairing.alert.cancel", default: "取消")) {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.tr("common.save", default: "保存")) {
                        let pattern = draft.modelPattern.trimmingCharacters(in: .whitespacesAndNewlines)
                        let provider = draft.providerId.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !pattern.isEmpty, !provider.isEmpty else { return }
                        let entry = ModelPricingEntryDTO(
                            modelPattern: pattern,
                            providerId: provider,
                            inputPerMMicroUsd: parseMicro(draft.inputUsd),
                            outputPerMMicroUsd: parseMicro(draft.outputUsd),
                            cacheReadPerMMicroUsd: parseMicro(draft.cacheReadUsd),
                            cacheWritePerMMicroUsd: parseMicro(draft.cacheWriteUsd),
                            reasoningPerMMicroUsd: parseMicro(draft.reasoningUsd),
                            isUserOverride: true,
                            effectiveFrom: TokenVaneFormatters.formatDayKey(Date())
                        )
                        Task {
                            if await onSave(entry) {
                                dismiss()
                            }
                        }
                    }
                }
            }
        }
    }

    private func rateField(_ title: String, text: Binding<String>) -> some View {
        HStack {
            Text(title)
            Spacer()
            TextField("0", text: text)
                .keyboardType(.decimalPad)
                .multilineTextAlignment(.trailing)
                .frame(width: 110)
        }
    }

    private func parseMicro(_ raw: String) -> Int64 {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let v = Double(trimmed), v.isFinite, v >= 0 else { return 0 }
        return Int64((v * 1_000_000.0).rounded())
    }
}
