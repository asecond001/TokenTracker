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
        case .monthly: return L10n.tr("analytics.mode.monthly", default: "月报")
        }
    }
}

private struct FlatTrendPoint: Identifiable {
    let seriesKey: String
    let seriesLabel: String
    let date: Date
    let bucketMs: Int64
    let tokens: Double
    let costMicroUsd: Int64

    var id: String { "\(seriesKey)::\(bucketMs)" }
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
        .background(SemanticColors.surfacePrimary)
        .sheet(item: $exportItem) { item in
            ShareSheetView(url: item.url)
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
        let scaleEntries = buildScaleEntries(from: report)
        let colorByKey = Dictionary(uniqueKeysWithValues: scaleEntries.map { ($0.key, $0.color) })
        let domainRange = xScaleDomain(buckets: report?.buckets ?? [], stepSec: stepSec)
        let tickDates = sampledTickDates(buckets: report?.buckets ?? [], stepSec: stepSec)

        return VStack(alignment: .leading, spacing: 14) {
            // Controls row
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(L10n.tr("analytics.dimension", default: "维度"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Picker(L10n.tr("analytics.dimension", default: "维度"), selection: $store.trendDimension) {
                        ForEach(StackDimensionDTO.allCases, id: \.self) { dim in
                            Text(dim.label).tag(dim)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 240)
                    .onChange(of: store.trendDimension) { _, _ in
                        selectedBucketMs = nil
                        selectedChartDate = nil
                        Task { await store.reloadTrendsOnly() }
                    }
                }

                HStack {
                    Text(L10n.tr("analytics.granularity", default: "粒度"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Picker(L10n.tr("analytics.granularity", default: "粒度"), selection: $store.trendGranularity) {
                        ForEach([TrendGranularityDTO.auto, .hourly, .daily, .weekly], id: \.self) { gran in
                            Text(gran.label).tag(gran)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 240)
                    .onChange(of: store.trendGranularity) { _, _ in
                        selectedBucketMs = nil
                        selectedChartDate = nil
                        Task { await store.reloadTrendsOnly() }
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)

            // Crosshair readout strip
            if let bucketMs = selectedBucketMs,
               let summary = bucketSummary(bucketMs: bucketMs, report: report) {
                HStack {
                    Text(TokenVaneFormatters.formatBucketTimestamp(bucketMs, isDateOnly: isDateOnly))
                        .font(.caption.weight(.semibold))
                        .tabularMetrics()
                    Spacer()
                    Text("\(TokenVaneFormatters.formatCompactTokens(summary.tokens)) · \(TokenVaneFormatters.formatCurrency(summary.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
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
                }
                .padding(.horizontal, 16)
            }

            // Frameless full-width Swift Chart with aligned bar centers, explicit domain, and chartXSelection
            if flatPoints.isEmpty {
                Text(L10n.tr("analytics.trends.waiting", default: "等待趋势数据"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 36)
            } else {
                GeometryReader { geo in
                    let barWidth = computedBarWidth(
                        chartWidth: geo.size.width,
                        buckets: report?.buckets ?? [],
                        stepMs: stepMs
                    )
                    Chart {
                        ForEach(flatPoints) { pt in
                            BarMark(
                                x: .value(L10n.tr("chart.axis.time", default: "时间"), pt.date),
                                y: .value(L10n.tr("chart.axis.tokens", default: "Tokens"), pt.tokens),
                                width: .fixed(barWidth)
                            )
                            .foregroundStyle(by: .value(L10n.tr("chart.axis.series", default: "系列"), pt.seriesLabel))
                        }

                        if let selectedBucketMs {
                            let center = Date(timeIntervalSince1970: Double(selectedBucketMs) / 1000.0 + stepSec * 0.5)
                            RuleMark(x: .value(L10n.tr("chart.axis.selected", default: "选中"), center))
                                .lineStyle(StrokeStyle(lineWidth: 1, dash: [3, 3]))
                                .foregroundStyle(Color.secondary)
                        }
                    }
                    .chartForegroundStyleScale(
                        domain: scaleEntries.map(\.label),
                        range: scaleEntries.map(\.color)
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
                                    Text(TokenVaneFormatters.formatCompactTokens(Int64(v)))
                                        .font(.caption2)
                                        .tabularMetrics()
                                }
                            }
                        }
                    }
                    .chartXSelection(value: $selectedChartDate)
                    .onChange(of: selectedChartDate) { _, newDate in
                        guard let newDate, let buckets = report?.buckets, !buckets.isEmpty else {
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
                .frame(height: 224)
                .padding(.horizontal, 16)
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
                    HStack(spacing: 8) {
                        Circle()
                            .fill(dotColor)
                            .frame(width: 7, height: 7)
                        Text(row.label)
                            .font(.caption.weight(.medium))
                            .lineLimit(1)
                        Spacer()
                        Text(TokenVaneFormatters.formatCompactTokens(row.currentTokens))
                            .font(.caption)
                            .tabularMetrics()
                        Text(formatDelta(row.deltaPct))
                            .font(.caption2.weight(.semibold))
                            .tabularMetrics()
                            .foregroundStyle(deltaColor(row.deltaPct))
                            .frame(width: 64, alignment: .trailing)
                    }
                    .padding(.vertical, 3)
                    HairlineDivider()
                }
            }
            .padding(.horizontal, 16)
        }
    }

    private var monthlySection: some View {
        let mo = store.monthly
        return VStack(alignment: .leading, spacing: 14) {
            HStack {
                Button {
                    Task { await store.shiftMonth(by: -1) }
                } label: {
                    Image(systemName: "chevron.left")
                        .padding(6)
                }

                Text(String(format: "%04d-%02d", store.selectedYear, store.selectedMonth))
                    .font(.headline)
                    .tabularMetrics()

                Button {
                    Task { await store.shiftMonth(by: 1) }
                } label: {
                    Image(systemName: "chevron.right")
                        .padding(6)
                }

                Spacer()

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

            HStack(spacing: 0) {
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.cost", default: "月度花费"),
                    value: TokenVaneFormatters.formatCurrency(mo?.apiEquivalentCostMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate)
                )
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.tokens", default: "月度 Tokens"),
                    value: TokenVaneFormatters.formatCompactTokens(mo?.monthTotalTokens ?? 0)
                )
                monthlySummaryCell(
                    title: L10n.tr("analytics.monthly.saved", default: "缓存节省"),
                    value: TokenVaneFormatters.formatCurrency(mo?.monthCacheSavedMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate)
                )
            }
            .padding(.horizontal, 16)

            HairlineDivider(insetLeading: 16)

            VStack(alignment: .leading, spacing: 8) {
                Text(L10n.tr("analytics.heatmap.title", default: "年度活跃"))
                    .font(.subheadline.weight(.semibold))
                Heatmap365Canvas(
                    cells: mo?.heatmap365d ?? [],
                    year: Int(store.selectedYear),
                    month: Int(store.selectedMonth)
                )
                .frame(height: 80)
            }
            .padding(.horizontal, 16)

            HairlineDivider(insetLeading: 16)

            VStack(alignment: .leading, spacing: 8) {
                Text(L10n.tr("analytics.ledger.title", default: "按日明细"))
                    .font(.subheadline.weight(.semibold))
                let rows = (mo?.dailyLedger ?? []).sorted { $0.date > $1.date }
                if rows.isEmpty {
                    Text(L10n.tr("analytics.ledger.waiting", default: "账单待同步"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(rows) { day in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
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

    private func flattenedPoints(from report: TrendsReportDTO?, stepSec: TimeInterval) -> [FlatTrendPoint] {
        guard let report else { return [] }
        var out: [FlatTrendPoint] = []
        for s in report.series.prefix(6) {
            for p in s.points where p.tokens > 0 {
                out.append(FlatTrendPoint(
                    seriesKey: s.key,
                    seriesLabel: s.label,
                    date: Date(timeIntervalSince1970: Double(p.bucketStartMs) / 1000.0 + stepSec * 0.5),
                    bucketMs: p.bucketStartMs,
                    tokens: Double(p.tokens),
                    costMicroUsd: p.costMicroUsd
                ))
            }
        }
        return out
    }

    private func bucketSummary(bucketMs: Int64, report: TrendsReportDTO?) -> (tokens: Int64, costMicroUsd: Int64)? {
        guard let report else { return nil }
        var tokens: Int64 = 0
        var cost: Int64 = 0
        for s in report.series {
            if let pt = s.points.first(where: { $0.bucketStartMs == bucketMs }) {
                tokens += pt.tokens
                cost += pt.costMicroUsd
            }
        }
        return (tokens, cost)
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

private struct Heatmap365Canvas: View {
    let cells: [HeatmapDayCellDTO]
    let year: Int
    let month: Int

    var body: some View {
        let lookup = buildTokenLookup(cells)
        let maxTokens = max(1, lookup.values.max() ?? 1)
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

                    let key = integerDateKey(cellDate, calendar: cal)
                    let tokens = lookup[key] ?? 0
                    let color: Color
                    if tokens <= 0 {
                        color = SemanticColors.hairline
                    } else {
                        let ratio = Double(tokens) / Double(maxTokens)
                        let opacity = 0.25 + min(1.0, ratio) * 0.75
                        color = SemanticColors.accentIndigo.opacity(opacity)
                    }
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

    private func buildTokenLookup(_ cells: [HeatmapDayCellDTO]) -> [Int: Int64] {
        var map: [Int: Int64] = [:]
        map.reserveCapacity(cells.count)
        for cell in cells {
            if let key = parseDateKey(cell.date) {
                map[key, default: 0] += cell.totalTokens
            }
        }
        return map
    }

    private func parseDateKey(_ raw: String) -> Int? {
        let parts = raw.split(separator: "-")
        guard parts.count == 3,
              let y = Int(parts[0]),
              let m = Int(parts[1]),
              let d = Int(parts[2]) else {
            return nil
        }
        return y * 10_000 + m * 100 + d
    }

    private func integerDateKey(_ date: Date, calendar: Calendar) -> Int {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return (c.year ?? 0) * 10_000 + (c.month ?? 0) * 100 + (c.day ?? 0)
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
