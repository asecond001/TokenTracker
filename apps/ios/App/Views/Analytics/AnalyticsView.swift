import Charts
import SwiftUI
import UIKit

private enum AnalyticsSubMode: String, CaseIterable, Identifiable {
    case trends
    case monthly

    var id: String { rawValue }

    var title: String {
        switch self {
        case .trends: return L10n.tr("analytics.mode.trends", default: "趋势")
        case .monthly: return L10n.tr("analytics.mode.monthly", default: "统计")
        }
    }
}

private enum LedgerSortMode: String, CaseIterable, Identifiable {
    case date
    case cost
    case tokens

    var id: String { rawValue }

    var label: String {
        switch self {
        case .date: return L10n.tr("analytics.ledger.sort.date", default: "日期")
        case .cost: return L10n.tr("analytics.ledger.sort.cost", default: "花费")
        case .tokens: return L10n.tr("analytics.ledger.sort.tokens", default: "Tokens")
        }
    }
}

private struct FlatTrendPoint: Identifiable {
    let seriesKey: String
    let seriesLabel: String
    let date: Date
    let bucketMs: Int64
    let value: Double
    let tokens: Int64
    let costMicroUsd: Int64
    let requests: Int64

    var id: String { "\(seriesKey)::\(bucketMs)" }
}

private struct PreviousOverlayPoint: Identifiable {
    let date: Date
    let bucketMs: Int64
    let value: Double

    var id: Int64 { bucketMs }
}

private struct SeriesScaleEntry {
    let key: String
    let label: String
    let color: Color
}

public struct AnalyticsView: View {
    @Bindable var store: AppStore
    @State private var subMode: AnalyticsSubMode = .trends
    @State private var selectedBucketMs: Int64?
    @State private var selectedChartDate: Date?
    @State private var exportItem: ExportFileItem?
    @State private var showingShareReceipt = false
    @State private var roiExpanded = true
    @State private var ledgerSort: LedgerSortMode = .date

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Picker(L10n.tr("analytics.picker.view", default: "视图"), selection: $subMode) {
                    ForEach(AnalyticsSubMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .accessibilityIdentifier("analytics.subModePicker")

                HairlineDivider()

                if subMode == .trends {
                    trendsSection
                } else {
                    monthlySection
                }
            }
            .padding(.bottom, 24)
        }
        .background(Color.clear)
        .sheet(item: $exportItem) { item in
            ShareSheetView(url: item.url)
        }
        .sheet(isPresented: $showingShareReceipt) {
            if let mo = store.monthly {
                ShareReceiptSheet(
                    report: mo,
                    currency: store.currency,
                    usdToCnyRate: store.usdToCnyRate,
                    periodLabel: store.resolvedDateSpanLabel
                )
            }
        }
    }

    private var trendsSection: some View {
        let report = store.trends
        let granularity = report?.effectiveGranularity ?? store.trendGranularity
        let stepSec = bucketDurationSeconds(for: granularity)
        let stepMs = Int64(stepSec * 1000.0)
        let isSubDay = (granularity == .hourly || granularity == .fifteenMinutes)
        let isDateOnly = !isSubDay
        let flatPoints = flattenedPoints(from: report, stepSec: stepSec)
        let overlayPoints = previousOverlayPoints(
            current: report,
            previous: store.previousPeriodTrends,
            stepSec: stepSec
        )
        let scaleEntries = buildScaleEntries(from: report)
        let colorByKey = Dictionary(uniqueKeysWithValues: scaleEntries.map { ($0.key, $0.color) })
        let domainRange = xScaleDomain(buckets: report?.buckets ?? [], stepSec: stepSec)
        let tickDates = sampledTickDates(buckets: report?.buckets ?? [], stepSec: stepSec)

        return VStack(alignment: .leading, spacing: 14) {
            trendsControls(scaleEntries: scaleEntries)

            // Crosshair readout strip
            if let bucketMs = selectedBucketMs,
               let summary = bucketSummary(bucketMs: bucketMs, report: report) {
                HStack {
                    Text(TokenVaneFormatters.formatBucketTimestamp(bucketMs, isDateOnly: isDateOnly))
                        .font(.caption.weight(.semibold))
                        .tabularMetrics()
                    Spacer()
                    Text("\(TokenVaneFormatters.formatCompactTokens(summary.tokens)) · \(TokenVaneFormatters.formatCurrency(summary.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate)) · \(summary.requests) req")
                        .font(.caption.weight(.semibold))
                        .tabularMetrics()
                        .foregroundStyle(SemanticColors.accentIndigo)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                .background(SemanticColors.surfaceSubtle)
            } else if let report {
                HStack {
                    Text(
                        L10n.trFormat(
                            "analytics.effective_granularity",
                            default: "粒度 · %@",
                            report.effectiveGranularity.label
                        )
                    )
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    Spacer()
                    if store.comparePreviousPeriod && store.timeRange != .all && !overlayPoints.isEmpty {
                        HStack(spacing: 4) {
                            Rectangle()
                                .stroke(style: StrokeStyle(lineWidth: 1.5, dash: [3, 3]))
                                .foregroundStyle(.secondary)
                                .frame(width: 16, height: 2)
                            Text(L10n.tr("trends.prev_period", default: "上期对照"))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .padding(.horizontal, 16)
            }

            if flatPoints.isEmpty {
                Text(L10n.tr("analytics.trends.waiting", default: "等待趋势数据"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 36)
            } else {
                trendsChart(
                    flatPoints: flatPoints,
                    overlayPoints: overlayPoints,
                    scaleEntries: scaleEntries,
                    domainRange: domainRange,
                    tickDates: tickDates,
                    stepSec: stepSec,
                    stepMs: stepMs,
                    isSubDay: isSubDay,
                    buckets: report?.buckets ?? []
                )
            }

            HairlineDivider(insetLeading: 16)

            // Period comparison flat table
            VStack(alignment: .leading, spacing: 8) {
                Text(L10n.tr("analytics.comparison.title", default: "环比对比"))
                    .font(.subheadline.weight(.semibold))
                let comparisons = report?.comparisons ?? []
                ForEach(Array(comparisons.enumerated()), id: \.element.id) { idx, row in
                    let dotColor = colorByKey[row.key]
                        ?? TokenVaneFormatters.seriesColor(
                            dimensionKey: store.trendDimension.rawValue,
                            seriesKey: row.key,
                            index: idx
                        )
                    let isFocused = store.focusedSeriesKey == row.key
                    let delta = store.trendMetric == .cost ? row.costDeltaPct : row.deltaPct

                    Button {
                        store.focusedSeriesKey = isFocused ? nil : row.key
                    } label: {
                        HStack(spacing: 8) {
                            Circle()
                                .fill(dotColor)
                                .frame(width: 7, height: 7)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(row.label)
                                    .font(.caption.weight(isFocused ? .bold : .medium))
                                    .foregroundStyle(.primary)
                                    .lineLimit(1)
                                Text("\(row.requests) req · 命中 \(TokenVaneFormatters.formatPercent(row.cacheHitRatio * 100, decimals: 0))")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .trailing, spacing: 2) {
                                Text(comparisonPrimaryValue(row))
                                    .font(.caption.weight(.medium))
                                    .tabularMetrics()
                                    .foregroundStyle(.primary)
                                Text(store.timeRange == .all ? "—" : formatDelta(delta))
                                    .font(.caption2.weight(.semibold))
                                    .tabularMetrics()
                                    .foregroundStyle(store.timeRange == .all ? Color.secondary : deltaColor(delta))
                            }
                        }
                        .padding(.vertical, 4)
                        .padding(.horizontal, isFocused ? 6 : 0)
                        .background(
                            isFocused ? SemanticColors.surfaceSubtle : Color.clear,
                            in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                        )
                    }
                    .buttonStyle(.plain)
                    HairlineDivider()
                }
            }
            .padding(.horizontal, 16)
        }
    }

    private func trendsControls(scaleEntries: [SeriesScaleEntry]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Picker(L10n.tr("trends.metric.label", default: "指标"), selection: $store.trendMetric) {
                ForEach(TrendMetricMode.allCases) { m in
                    Text(m.label).tag(m)
                }
            }
            .pickerStyle(.segmented)

            HStack {
                Picker(L10n.tr("analytics.dimension", default: "维度"), selection: $store.trendDimension) {
                    ForEach(StackDimensionDTO.allCases, id: \.self) { dim in
                        Text(dim.label).tag(dim)
                    }
                }
                .pickerStyle(.segmented)
                .onChange(of: store.trendDimension) { _, _ in
                    selectedBucketMs = nil
                    selectedChartDate = nil
                    store.focusedSeriesKey = nil
                    Task { await store.reloadTrendsOnly() }
                }

                Picker(L10n.tr("trends.style.label", default: "图表"), selection: $store.trendChartStyle) {
                    ForEach(TrendChartStyleMode.allCases) { st in
                        Text(st.label).tag(st)
                    }
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 112)
            }

            HStack {
                Picker(L10n.tr("analytics.granularity", default: "粒度"), selection: $store.trendGranularity) {
                    ForEach(TrendGranularityDTO.allCases, id: \.self) { gran in
                        Text(gran.label).tag(gran)
                    }
                }
                .pickerStyle(.segmented)
                .onChange(of: store.trendGranularity) { _, _ in
                    selectedBucketMs = nil
                    selectedChartDate = nil
                    Task { await store.reloadTrendsOnly() }
                }

                if store.timeRange != .all {
                    Toggle(
                        L10n.tr("trends.compare_prev", default: "对比上期"),
                        isOn: Binding(
                            get: { store.comparePreviousPeriod },
                            set: { val in Task { await store.setComparePreviousPeriod(val) } }
                        )
                    )
                    .font(.caption)
                    .fixedSize()
                }
            }

            if !scaleEntries.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        Button {
                            store.focusedSeriesKey = nil
                        } label: {
                            Text(L10n.tr("trends.series.all", default: "全部系列"))
                                .font(.caption2.weight(store.focusedSeriesKey == nil ? .semibold : .regular))
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(
                                    store.focusedSeriesKey == nil
                                        ? SemanticColors.accentIndigo.opacity(0.15)
                                        : SemanticColors.surfaceSubtle,
                                    in: Capsule()
                                )
                        }
                        .buttonStyle(.plain)

                        ForEach(scaleEntries, id: \.key) { entry in
                            let selected = store.focusedSeriesKey == entry.key
                            Button {
                                store.focusedSeriesKey = selected ? nil : entry.key
                            } label: {
                                HStack(spacing: 4) {
                                    Circle()
                                        .fill(entry.color)
                                        .frame(width: 6, height: 6)
                                    Text(entry.label)
                                        .font(.caption2.weight(selected ? .semibold : .regular))
                                        .lineLimit(1)
                                }
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(
                                    selected ? entry.color.opacity(0.18) : SemanticColors.surfaceSubtle,
                                    in: Capsule()
                                )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
    }

    @ViewBuilder
    private func trendsChart(
        flatPoints: [FlatTrendPoint],
        overlayPoints: [PreviousOverlayPoint],
        scaleEntries: [SeriesScaleEntry],
        domainRange: ClosedRange<Date>,
        tickDates: [Date],
        stepSec: TimeInterval,
        stepMs: Int64,
        isSubDay: Bool,
        buckets: [Int64]
    ) -> some View {
        let activeEntries = store.focusedSeriesKey.flatMap { focused in
            let matched = scaleEntries.filter { $0.key == focused }
            return matched.isEmpty ? nil : matched
        } ?? scaleEntries

        GeometryReader { geo in
            let barWidth = computedBarWidth(
                chartWidth: geo.size.width,
                buckets: buckets,
                stepMs: stepMs
            )
            Chart {
                trendsMarks(
                    flatPoints: flatPoints,
                    overlayPoints: overlayPoints,
                    barWidth: barWidth,
                    stepSec: stepSec
                )
            }
            .chartForegroundStyleScale(
                domain: activeEntries.map(\.label),
                range: activeEntries.map(\.color)
            )
            .chartXScale(domain: domainRange)
            .chartLegend(position: .bottom, alignment: .leading)
            .chartXAxis {
                AxisMarks(values: tickDates) { value in
                    AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5))
                        .foregroundStyle(SemanticColors.hairline)
                    AxisValueLabel {
                        if let d = value.as(Date.self) {
                            let bucketStart = d.addingTimeInterval(-stepSec * 0.5)
                            Text(TokenVaneFormatters.formatChartAxisDate(bucketStart, subDay: isSubDay))
                                .font(.caption2)
                                .tabularMetrics()
                        }
                    }
                }
            }
            .chartYAxis {
                AxisMarks(position: .trailing) { value in
                    AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5))
                        .foregroundStyle(SemanticColors.hairline)
                    AxisValueLabel {
                        if let v = value.as(Double.self) {
                            Text(formatChartYAxisValue(v))
                                .font(.caption2)
                                .tabularMetrics()
                        }
                    }
                }
            }
            .chartXSelection(value: $selectedChartDate)
            .onChange(of: selectedChartDate) { _, newDate in
                guard let newDate, !buckets.isEmpty else {
                    selectedBucketMs = nil
                    return
                }
                let targetMs = Int64(newDate.timeIntervalSince1970 * 1000.0)
                let halfStep = stepMs / 2
                let nearest = buckets.min(by: {
                    abs(($0 + halfStep) - targetMs) < abs(($1 + halfStep) - targetMs)
                })
                if let nearest, nearest != selectedBucketMs {
                    selectedBucketMs = nearest
                    let gen = UIImpactFeedbackGenerator(style: .light)
                    gen.impactOccurred()
                }
            }
        }
        .frame(height: 236)
        .padding(.horizontal, 16)
    }

    @ChartContentBuilder
    private func trendsMarks(
        flatPoints: [FlatTrendPoint],
        overlayPoints: [PreviousOverlayPoint],
        barWidth: CGFloat,
        stepSec: TimeInterval
    ) -> some ChartContent {
        if store.trendChartStyle == .bar {
            ForEach(flatPoints) { pt in
                BarMark(
                    x: .value(L10n.tr("chart.axis.time", default: "时间"), pt.date),
                    y: .value(store.trendMetric.label, pt.value),
                    width: .fixed(barWidth)
                )
                .foregroundStyle(by: .value(L10n.tr("chart.axis.series", default: "系列"), pt.seriesLabel))
            }
        } else {
            ForEach(flatPoints) { pt in
                LineMark(
                    x: .value(L10n.tr("chart.axis.time", default: "时间"), pt.date),
                    y: .value(store.trendMetric.label, pt.value),
                    series: .value(L10n.tr("chart.axis.series", default: "系列"), pt.seriesLabel)
                )
                .interpolationMethod(.monotone)
                .lineStyle(StrokeStyle(lineWidth: 2))
                .foregroundStyle(by: .value(L10n.tr("chart.axis.series", default: "系列"), pt.seriesLabel))
            }
        }

        if store.comparePreviousPeriod && store.timeRange != .all {
            ForEach(overlayPoints) { prevPt in
                LineMark(
                    x: .value(L10n.tr("chart.axis.time", default: "时间"), prevPt.date),
                    y: .value(store.trendMetric.label, prevPt.value),
                    series: .value("Comparison", "__previous__")
                )
                .interpolationMethod(.monotone)
                .lineStyle(StrokeStyle(lineWidth: 1.5, dash: [4, 4]))
                .foregroundStyle(Color.secondary.opacity(0.65))
            }
        }

        if let selectedBucketMs {
            let center = Date(timeIntervalSince1970: Double(selectedBucketMs) / 1000.0 + stepSec * 0.5)
            RuleMark(x: .value(L10n.tr("chart.axis.selected", default: "选中"), center))
                .lineStyle(StrokeStyle(lineWidth: 1, dash: [3, 3]))
                .foregroundStyle(Color.secondary)
        }
    }

    private var monthlySection: some View {
        let mo = store.monthly
        let feeMicro = mo?.subscriptionPeriodFeeMicroUsd ?? mo?.subscriptionMonthlyFeeMicroUsd ?? 0
        let apiMicro = mo?.apiEquivalentCostMicroUsd ?? 0
        let netSavedMicro = max(Int64(0), apiMicro - feeMicro)

        return VStack(alignment: .leading, spacing: 14) {
            // Range & Action header
            HStack(spacing: 8) {
                Button {
                    Task { await store.shiftMonth(by: -1) }
                } label: {
                    Image(systemName: "chevron.left")
                        .padding(6)
                }

                VStack(alignment: .leading, spacing: 1) {
                    Text(store.resolvedDateSpanLabel)
                        .font(.subheadline.weight(.semibold))
                        .tabularMetrics()
                        .lineLimit(1)
                    if let mo {
                        Text(String(format: "%04d-%02d", mo.year, mo.month))
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                    }
                }

                Button {
                    Task { await store.shiftMonth(by: 1) }
                } label: {
                    Image(systemName: "chevron.right")
                        .padding(6)
                }

                Spacer()

                if mo != nil {
                    Button {
                        showingShareReceipt = true
                    } label: {
                        Label(L10n.tr("analytics.monthly.receipt", default: "小票"), systemImage: "doc.text.image")
                            .font(.caption.weight(.medium))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                    }
                    .buttonStyle(.plain)
                }

                Menu {
                    Button(L10n.tr("analytics.monthly.export_csv", default: "导出 CSV")) {
                        Task {
                            if let url = await store.generateMonthlyExportFile(format: "csv") {
                                exportItem = ExportFileItem(url: url)
                            }
                        }
                    }
                    Button(L10n.tr("analytics.monthly.export_json", default: "导出 JSON")) {
                        Task {
                            if let url = await store.generateMonthlyExportFile(format: "json") {
                                exportItem = ExportFileItem(url: url)
                            }
                        }
                    }
                } label: {
                    Label(L10n.tr("analytics.monthly.export", default: "导出"), systemImage: "square.and.arrow.up")
                        .font(.caption.weight(.medium))
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)

            // 4 Summary KPI cells
            LazyVGrid(
                columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)],
                spacing: 12
            ) {
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.cost", default: "等效花费"),
                    value: TokenVaneFormatters.formatCurrency(apiMicro, currency: store.currency, rateString: store.usdToCnyRate)
                )
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.tokens", default: "Token 合计"),
                    value: TokenVaneFormatters.formatCompactTokens(mo?.monthTotalTokens ?? 0)
                )
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.requests", default: "请求次数"),
                    value: TokenVaneFormatters.formatExactInt(mo?.monthTotalRequests ?? 0)
                )
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.saved", default: "缓存节省"),
                    value: TokenVaneFormatters.formatCurrency(mo?.monthCacheSavedMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate)
                )
            }
            .padding(.horizontal, 16)

            // Provider & Model Bill Shares
            if !(mo?.providerShares.isEmpty ?? true) || !(mo?.modelShares.isEmpty ?? true) {
                HairlineDivider(insetLeading: 16)
                billSharesSection(mo: mo)
            }

            HairlineDivider(insetLeading: 16)

            // 365-day Quartile Heatmap
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(L10n.tr("analytics.heatmap.title", default: "年度活跃"))
                        .font(.subheadline.weight(.semibold))
                    Spacer()
                    HStack(spacing: 3) {
                        Text(L10n.tr("analytics.heatmap.less", default: "少"))
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        ForEach(0..<5, id: \.self) { level in
                            RoundedRectangle(cornerRadius: 1.5)
                                .fill(heatmapColor(for: level))
                                .frame(width: 8, height: 8)
                        }
                        Text(L10n.tr("analytics.heatmap.more", default: "多"))
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
                Heatmap365Canvas(
                    cells: mo?.heatmap365d ?? [],
                    year: Int(mo?.year ?? store.selectedYear),
                    month: Int(mo?.month ?? store.selectedMonth)
                )
                .frame(height: 82)
            }
            .padding(.horizontal, 16)

            HairlineDivider(insetLeading: 16)

            // Collapsible Subscription ROI
            VStack(alignment: .leading, spacing: 10) {
                Button {
                    roiExpanded.toggle()
                } label: {
                    HStack {
                        Text(L10n.tr("analytics.roi.title", default: "订阅 ROI 测算"))
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                        Spacer()
                        if let roi = mo?.roiMultiplier, roi.isFinite, feeMicro > 0 {
                            Text(String(format: "%.2f×", locale: Locale(identifier: "en_US_POSIX"), roi))
                                .font(.caption.weight(.semibold))
                                .tabularMetrics()
                                .foregroundStyle(SemanticColors.statusLive)
                        }
                        Image(systemName: roiExpanded ? "chevron.up" : "chevron.down")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
                .buttonStyle(.plain)

                if roiExpanded {
                    if feeMicro > 0 {
                        LazyVGrid(
                            columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)],
                            spacing: 10
                        ) {
                            monthlySummaryCell(
                                title: L10n.tr("analytics.roi.period_fee", default: "周期订阅实付"),
                                value: TokenVaneFormatters.formatCurrency(feeMicro, currency: store.currency, rateString: store.usdToCnyRate)
                            )
                            monthlySummaryCell(
                                title: L10n.tr("analytics.roi.net_saved", default: "净等效盈余"),
                                value: TokenVaneFormatters.formatCurrency(netSavedMicro, currency: store.currency, rateString: store.usdToCnyRate)
                            )
                        }
                    } else {
                        Text(L10n.tr("analytics.roi.empty", default: "暂未配置付费订阅计划，可前往「订阅」页添加"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding(.horizontal, 16)

            HairlineDivider(insetLeading: 16)

            // Daily Ledger
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(L10n.tr("analytics.ledger.title", default: "按日明细"))
                        .font(.subheadline.weight(.semibold))
                    Spacer()
                    Picker(L10n.tr("analytics.ledger.sort", default: "排序"), selection: $ledgerSort) {
                        ForEach(LedgerSortMode.allCases) { mode in
                            Text(mode.label).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 190)
                }

                let rows = sortedLedgerRows(mo?.dailyLedger ?? [])
                if rows.isEmpty {
                    Text(L10n.tr("analytics.ledger.waiting", default: "账单待同步"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(rows) { day in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                HStack(spacing: 6) {
                                    Text(day.date)
                                        .font(.caption.weight(.medium))
                                        .tabularMetrics()
                                    if let topModel = day.topModel {
                                        Text(topModel)
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                            .lineLimit(1)
                                    }
                                }
                                Text("In \(TokenVaneFormatters.formatCompactTokens(day.inputTokens)) · Out \(TokenVaneFormatters.formatCompactTokens(day.outputTokens)) · Cache \(TokenVaneFormatters.formatCompactTokens(day.cacheReadTokens))")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .trailing, spacing: 2) {
                                Text(TokenVaneFormatters.formatCurrency(day.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                    .font(.caption.weight(.semibold))
                                    .tabularMetrics()
                                Text("\(TokenVaneFormatters.formatCompactTokens(day.totalTokens)) · \(day.requests) req")
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 3)
                        HairlineDivider()
                    }
                }
            }
            .padding(.horizontal, 16)
        }
    }

    @ViewBuilder
    private func billSharesSection(mo: MonthlyReportDTO?) -> some View {
        let providers = mo?.providerShares ?? []
        let models = mo?.modelShares ?? []
        let segments = providers.prefix(6).map {
            ShareBarSegment(
                id: $0.key,
                label: $0.label,
                sharePercent: $0.costSharePercent,
                color: TokenVaneFormatters.providerColor($0.key)
            )
        }

        VStack(alignment: .leading, spacing: 10) {
            Text(L10n.tr("analytics.shares.title", default: "账单构成"))
                .font(.subheadline.weight(.semibold))

            if !segments.isEmpty {
                InlineShareBar(segments: segments, height: 6)
            }

            if !providers.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.tr("overview.shares.by_provider", default: "服务商"))
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.secondary)
                    ForEach(providers.prefix(5)) { item in
                        HStack(spacing: 8) {
                            Circle()
                                .fill(TokenVaneFormatters.providerColor(item.key))
                                .frame(width: 7, height: 7)
                            Text(item.label)
                                .font(.caption)
                                .lineLimit(1)
                            Spacer()
                            Text(TokenVaneFormatters.formatPercent(item.costSharePercent))
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                            Text(TokenVaneFormatters.formatCurrency(item.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.caption.weight(.medium))
                                .tabularMetrics()
                                .frame(minWidth: 64, alignment: .trailing)
                        }
                    }
                }
            }

            if !models.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.tr("overview.shares.top_models", default: "Top 模型"))
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.secondary)
                    ForEach(models.prefix(5)) { item in
                        HStack(spacing: 8) {
                            Text(item.label)
                                .font(.caption)
                                .lineLimit(1)
                            Spacer()
                            Text(TokenVaneFormatters.formatCompactTokens(item.tokens))
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                            Text(TokenVaneFormatters.formatCurrency(item.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.caption.weight(.medium))
                                .tabularMetrics()
                                .frame(minWidth: 64, alignment: .trailing)
                        }
                    }
                }
                .padding(.top, 4)
            }
        }
        .padding(.horizontal, 16)
    }

    private func sortedLedgerRows(_ rows: [MonthlyDailyLedgerRowDTO]) -> [MonthlyDailyLedgerRowDTO] {
        switch ledgerSort {
        case .date: return rows.sorted { $0.date > $1.date }
        case .cost: return rows.sorted { $0.costMicroUsd > $1.costMicroUsd }
        case .tokens: return rows.sorted { $0.totalTokens > $1.totalTokens }
        }
    }

    private func monthlySummaryCell(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
                .minimumScaleFactor(0.75)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func comparisonPrimaryValue(_ row: TrendComparisonRowDTO) -> String {
        switch store.trendMetric {
        case .tokens: return TokenVaneFormatters.formatCompactTokens(row.currentTokens)
        case .cost: return TokenVaneFormatters.formatCurrency(row.currentCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate)
        case .cacheHit: return TokenVaneFormatters.formatPercent(row.cacheHitRatio * 100.0)
        case .requests: return TokenVaneFormatters.formatExactInt(row.requests)
        }
    }

    private func formatChartYAxisValue(_ v: Double) -> String {
        switch store.trendMetric {
        case .tokens:
            return TokenVaneFormatters.formatCompactTokens(Int64(v))
        case .cost:
            let symbol = store.currency.uppercased() == "CNY" ? "¥" : "$"
            return String(format: "%@%.2f", locale: Locale(identifier: "en_US_POSIX"), symbol, v)
        case .cacheHit:
            return String(format: "%.0f%%", locale: Locale(identifier: "en_US_POSIX"), v)
        case .requests:
            return TokenVaneFormatters.formatExactInt(Int64(v.rounded()))
        }
    }

    private func bucketDurationSeconds(for granularity: TrendGranularityDTO) -> TimeInterval {
        switch granularity {
        case .fifteenMinutes: return 900.0
        case .auto, .hourly: return 3600.0
        case .daily: return 86_400.0
        case .weekly: return 7.0 * 86_400.0
        }
    }

    private func xScaleDomain(buckets: [Int64], stepSec: TimeInterval) -> ClosedRange<Date> {
        guard let first = buckets.first, let last = buckets.last else {
            let now = Date()
            return now...now.addingTimeInterval(stepSec)
        }
        let start = Date(timeIntervalSince1970: Double(first) / 1000.0)
        let end = Date(timeIntervalSince1970: Double(last) / 1000.0 + stepSec)
        return start...end
    }

    private func sampledTickDates(buckets: [Int64], stepSec: TimeInterval) -> [Date] {
        guard !buckets.isEmpty else { return [] }
        let maxTicks = 5
        let indices: [Int]
        if buckets.count <= maxTicks {
            indices = Array(buckets.indices)
        } else {
            var picked = Set<Int>([0, buckets.count - 1])
            let step = Double(buckets.count - 1) / Double(maxTicks - 1)
            for i in 1..<(maxTicks - 1) {
                picked.insert(Int((Double(i) * step).rounded()))
            }
            indices = picked.sorted()
        }
        return indices.map { idx in
            Date(timeIntervalSince1970: Double(buckets[idx]) / 1000.0 + stepSec * 0.5)
        }
    }

    private func computedBarWidth(chartWidth: CGFloat, buckets: [Int64], stepMs: Int64) -> CGFloat {
        let plotWidth = max(120.0, chartWidth - 56.0)
        guard let first = buckets.first, let last = buckets.last, stepMs > 0 else {
            return 18.0
        }
        let spanSlots = max(1, Int((last - first) / stepMs) + 1)
        let raw = (plotWidth / CGFloat(spanSlots)) * 0.58
        return min(42.0, max(3.0, raw))
    }

    private func buildScaleEntries(from report: TrendsReportDTO?) -> [SeriesScaleEntry] {
        guard let report else { return [] }
        let dim = store.trendDimension.rawValue
        var seenLabels = Set<String>()
        var result: [SeriesScaleEntry] = []
        for (idx, s) in report.series.prefix(6).enumerated() {
            if seenLabels.insert(s.label).inserted {
                result.append(SeriesScaleEntry(
                    key: s.key,
                    label: s.label,
                    color: TokenVaneFormatters.seriesColor(dimensionKey: dim, seriesKey: s.key, index: idx)
                ))
            }
        }
        return result
    }

    private func metricValue(for pt: TrendPointDTO) -> Double {
        switch store.trendMetric {
        case .tokens:
            return Double(pt.tokens)
        case .cost:
            return TokenVaneFormatters.convertMicroUsd(
                pt.costMicroUsd,
                currency: store.currency,
                rateString: store.usdToCnyRate
            )
        case .cacheHit:
            return pt.cacheHitRatio * 100.0
        case .requests:
            return Double(pt.requests)
        }
    }

    private func flattenedPoints(from report: TrendsReportDTO?, stepSec: TimeInterval) -> [FlatTrendPoint] {
        guard let report else { return [] }
        let selectedSeries = report.series.prefix(6).filter { s in
            store.focusedSeriesKey == nil || store.focusedSeriesKey == s.key
        }
        var out: [FlatTrendPoint] = []
        for s in selectedSeries {
            for p in s.points where (p.tokens > 0 || p.requests > 0 || p.costMicroUsd > 0 || store.trendChartStyle == .line) {
                let val = metricValue(for: p)
                if store.trendChartStyle == .bar && val <= 0 { continue }
                out.append(FlatTrendPoint(
                    seriesKey: s.key,
                    seriesLabel: s.label,
                    date: Date(timeIntervalSince1970: Double(p.bucketStartMs) / 1000.0 + stepSec * 0.5),
                    bucketMs: p.bucketStartMs,
                    value: val,
                    tokens: p.tokens,
                    costMicroUsd: p.costMicroUsd,
                    requests: p.requests
                ))
            }
        }
        return out
    }

    private func previousOverlayPoints(
        current: TrendsReportDTO?,
        previous: TrendsReportDTO?,
        stepSec: TimeInterval
    ) -> [PreviousOverlayPoint] {
        guard store.comparePreviousPeriod,
              store.timeRange != .all,
              let current,
              let previous,
              !current.buckets.isEmpty,
              !previous.buckets.isEmpty else {
            return []
        }

        let targetSeries = previous.series.filter { s in
            store.focusedSeriesKey == nil || store.focusedSeriesKey == s.key
        }
        let pointMaps: [[Int64: TrendPointDTO]] = targetSeries.map { s in
            Dictionary(uniqueKeysWithValues: s.points.map { ($0.bucketStartMs, $0) })
        }

        let count = min(current.buckets.count, previous.buckets.count)
        var out: [PreviousOverlayPoint] = []
        out.reserveCapacity(count)
        for i in 0..<count {
            let curMs = current.buckets[i]
            let prevMs = previous.buckets[i]
            let date = Date(timeIntervalSince1970: Double(curMs) / 1000.0 + stepSec * 0.5)

            if store.trendMetric == .cacheHit {
                var sumRatio = 0.0
                var samples = 0
                for map in pointMaps {
                    if let pt = map[prevMs], pt.tokens > 0 {
                        sumRatio += pt.cacheHitRatio * 100.0
                        samples += 1
                    }
                }
                let avg = samples > 0 ? sumRatio / Double(samples) : 0.0
                out.append(PreviousOverlayPoint(date: date, bucketMs: curMs, value: avg))
            } else {
                var total = 0.0
                for map in pointMaps {
                    if let pt = map[prevMs] {
                        total += metricValue(for: pt)
                    }
                }
                out.append(PreviousOverlayPoint(date: date, bucketMs: curMs, value: total))
            }
        }
        return out
    }

    private func bucketSummary(bucketMs: Int64, report: TrendsReportDTO?) -> (tokens: Int64, costMicroUsd: Int64, requests: Int64)? {
        guard let report else { return nil }
        var tokens: Int64 = 0
        var cost: Int64 = 0
        var requests: Int64 = 0
        for s in report.series {
            if let focused = store.focusedSeriesKey, s.key != focused { continue }
            if let pt = s.points.first(where: { $0.bucketStartMs == bucketMs }) {
                tokens += pt.tokens
                cost += pt.costMicroUsd
                requests += pt.requests
            }
        }
        return (tokens, cost, requests)
    }

    private func formatDelta(_ delta: Double?) -> String {
        guard let delta, delta.isFinite else { return "—" }
        let prefix = delta > 0 ? "+" : ""
        return String(format: "%@%.1f%%", locale: Locale(identifier: "en_US_POSIX"), prefix, delta)
    }

    private func deltaColor(_ delta: Double?) -> Color {
        guard let delta, delta.isFinite else { return .secondary }
        if delta > 0 { return SemanticColors.statusWarning }
        if delta < 0 { return SemanticColors.statusLive }
        return .secondary
    }
}

private func heatmapColor(for level: Int) -> Color {
    switch level {
    case 1: return SemanticColors.accentIndigo.opacity(0.25)
    case 2: return SemanticColors.accentIndigo.opacity(0.45)
    case 3: return SemanticColors.accentIndigo.opacity(0.70)
    case 4: return SemanticColors.accentIndigo.opacity(0.96)
    default: return SemanticColors.hairline
    }
}

private struct Heatmap365Canvas: View {
    let cells: [HeatmapDayCellDTO]
    let year: Int
    let month: Int

    var body: some View {
        let levelLookup = ReceiptModelBuilder.computeHeatmapIntensityLevels(cells)
        let cal = Calendar(identifier: .gregorian)
        let endDate = resolvedEndDate(calendar: cal)
        let endWeekdayRow = (cal.component(.weekday, from: endDate) + 5) % 7
        let lastWeekMonday = cal.date(byAdding: .day, value: -endWeekdayRow, to: endDate) ?? endDate
        let firstWeekMonday = cal.date(byAdding: .day, value: -52 * 7, to: lastWeekMonday) ?? endDate
        let monthLabels = buildMonthLabels(firstWeekMonday: firstWeekMonday, endDate: endDate, calendar: cal)

        Canvas { context, size in
            let cols = 53
            let rows = 7
            let topMargin: CGFloat = 14
            let gridOriginX: CGFloat = 16
            let spacing: CGFloat = 2
            let cellW = max(2, (size.width - gridOriginX - CGFloat(cols - 1) * spacing) / CGFloat(cols))
            let cellH = max(2, (size.height - topMargin - CGFloat(rows - 1) * spacing) / CGFloat(rows))
            let cellSide = min(cellW, cellH)

            let weekdayMarks: [(row: Int, text: String)] = [
                (0, L10n.tr("analytics.heatmap.mon", default: "一")),
                (2, L10n.tr("analytics.heatmap.wed", default: "三")),
                (4, L10n.tr("analytics.heatmap.fri", default: "五"))
            ]
            for mark in weekdayMarks {
                let centerY = topMargin + CGFloat(mark.row) * (cellSide + spacing) + cellSide * 0.5
                let resolved = context.resolve(
                    Text(mark.text)
                        .font(.system(size: 9))
                        .foregroundStyle(.secondary)
                )
                context.draw(resolved, at: CGPoint(x: 0, y: centerY), anchor: .leading)
            }

            for label in monthLabels {
                let x = gridOriginX + CGFloat(label.col) * (cellSide + spacing)
                let resolved = context.resolve(
                    Text(label.text)
                        .font(.system(size: 9))
                        .foregroundStyle(.secondary)
                )
                context.draw(resolved, at: CGPoint(x: x, y: 0), anchor: .topLeading)
            }

            for col in 0..<cols {
                for row in 0..<rows {
                    let dayOffset = col * 7 + row
                    guard let cellDate = cal.date(byAdding: .day, value: dayOffset, to: firstWeekMonday),
                          cellDate <= endDate else {
                        continue
                    }
                    let x = gridOriginX + CGFloat(col) * (cellSide + spacing)
                    let y = topMargin + CGFloat(row) * (cellSide + spacing)
                    let rect = CGRect(x: x, y: y, width: cellSide, height: cellSide)

                    let key = ymdString(cellDate, calendar: cal)
                    let level = levelLookup[key] ?? 0
                    let color = heatmapColor(for: level)
                    context.fill(Path(roundedRect: rect, cornerRadius: 1.5), with: .color(color))
                }
            }
        }
    }

    private func resolvedEndDate(calendar: Calendar) -> Date {
        let today = calendar.startOfDay(for: Date())
        var comps = DateComponents()
        comps.year = year
        comps.month = month
        comps.day = 1
        guard let firstOfMonth = calendar.date(from: comps),
              let nextMonth = calendar.date(byAdding: .month, value: 1, to: firstOfMonth),
              let lastOfMonth = calendar.date(byAdding: .day, value: -1, to: nextMonth) else {
            return today
        }
        return min(lastOfMonth, today)
    }

    private func ymdString(_ date: Date, calendar: Calendar) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    private func buildMonthLabels(firstWeekMonday: Date, endDate: Date, calendar: Calendar) -> [(col: Int, text: String)] {
        let symbols = calendar.shortMonthSymbols
        var labels: [(col: Int, text: String)] = []
        var prevMonth = -1
        var lastLabelCol = -4
        for col in 0..<53 {
            guard let colDate = calendar.date(byAdding: .day, value: col * 7, to: firstWeekMonday),
                  colDate <= endDate else {
                continue
            }
            let m = calendar.component(.month, from: colDate)
            if m != prevMonth {
                prevMonth = m
                if col - lastLabelCol >= 3 && (53 - col) >= 2 {
                    let symbol = (m >= 1 && m <= symbols.count) ? symbols[m - 1] : "\(m)"
                    labels.append((col: col, text: symbol))
                    lastLabelCol = col
                }
            }
        }
        return labels
    }
}

private struct ExportFileItem: Identifiable {
    let id = UUID()
    let url: URL
}

private struct ShareSheetView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}
