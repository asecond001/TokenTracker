import Charts
import SwiftUI
import UIKit

private enum AnalyticsSubMode: String, CaseIterable, Identifiable {
    case trends = "趋势拆解"
    case monthly = "月度账单"

    var id: String { rawValue }
}

private struct FlatTrendPoint: Identifiable {
    let seriesKey: String
    let seriesLabel: String
    let date: Date
    let bucketMs: Int64
    let tokens: Double
    let costMicroUsd: Int64
    let color: Color

    var id: String { "\(seriesKey)::\(bucketMs)" }
}

public struct AnalyticsView: View {
    @Bindable var store: AppStore
    @State private var subMode: AnalyticsSubMode = .trends
    @State private var selectedBucketMs: Int64?
    @State private var exportItem: ExportFileItem?

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Picker("视图", selection: $subMode) {
                    ForEach(AnalyticsSubMode.allCases) { mode in
                        Text(mode.rawValue).tag(mode)
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
        let flatPoints = flattenedPoints(from: report)

        return VStack(alignment: .leading, spacing: 14) {
            // Controls row
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("拆解维度")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Picker("维度", selection: $store.trendDimension) {
                        ForEach(StackDimensionDTO.allCases, id: \.self) { dim in
                            Text(dim.label).tag(dim)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 240)
                    .onChange(of: store.trendDimension) { _, _ in
                        Task { await store.reloadAllData() }
                    }
                }

                HStack {
                    Text("时间粒度")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Picker("粒度", selection: $store.trendGranularity) {
                        ForEach([TrendGranularityDTO.auto, .hourly, .daily, .weekly], id: \.self) { gran in
                            Text(gran.label).tag(gran)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 240)
                    .onChange(of: store.trendGranularity) { _, _ in
                        Task { await store.reloadAllData() }
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)

            // Crosshair readout strip
            if let bucketMs = selectedBucketMs,
               let summary = bucketSummary(bucketMs: bucketMs, report: report) {
                HStack {
                    Text(formatBucketTimestamp(bucketMs, granularity: report?.effectiveGranularity ?? .hourly))
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
                    Text("生效粒度: \(report.effectiveGranularity.label) · 长按或拖动图表查看时段详情")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer()
                }
                .padding(.horizontal, 16)
            }

            // Frameless full-width Swift Chart
            if flatPoints.isEmpty {
                Text("当前筛选条件暂无趋势序列")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 36)
            } else {
                Chart(flatPoints) { pt in
                    BarMark(
                        x: .value("时间", pt.date),
                        y: .value("Tokens", pt.tokens)
                    )
                    .foregroundStyle(by: .value("系列", pt.seriesLabel))

                    if let selectedBucketMs {
                        RuleMark(x: .value("选中", Date(timeIntervalSince1970: Double(selectedBucketMs) / 1000.0)))
                            .lineStyle(StrokeStyle(lineWidth: 1, dash: [3, 3]))
                            .foregroundStyle(Color.secondary)
                    }
                }
                .chartLegend(position: .bottom, alignment: .leading)
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
                .chartOverlay { proxy in
                    GeometryReader { geo in
                        Rectangle()
                            .fill(.clear)
                            .contentShape(Rectangle())
                            .gesture(
                                DragGesture(minimumDistance: 0)
                                    .onChanged { value in
                                        guard let plotFrame = proxy.plotFrame else { return }
                                        let origin = geo[plotFrame].origin
                                        let xPos = value.location.x - origin.x
                                        guard let date: Date = proxy.value(atX: xPos),
                                              let buckets = report?.buckets,
                                              !buckets.isEmpty else {
                                            return
                                        }
                                        let targetMs = Int64(date.timeIntervalSince1970 * 1000)
                                        let nearest = buckets.min(by: { abs($0 - targetMs) < abs($1 - targetMs) })
                                        if let nearest, nearest != selectedBucketMs {
                                            selectedBucketMs = nearest
                                            let gen = UIImpactFeedbackGenerator(style: .light)
                                            gen.impactOccurred()
                                        }
                                    }
                                    .onEnded { _ in
                                        selectedBucketMs = nil
                                    }
                            )
                    }
                }
                .frame(height: 220)
                .padding(.horizontal, 16)
            }

            HairlineDivider(insetLeading: 16)

            // Period comparison flat table
            VStack(alignment: .leading, spacing: 8) {
                Text("环比对比（当期 vs 上一周期）")
                    .font(.subheadline.weight(.semibold))
                ForEach(report?.comparisons ?? []) { row in
                    HStack {
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
                    Button("导出 CSV 账单") {
                        Task {
                            if let url = await store.generateMonthlyExportFile(format: "csv") {
                                exportItem = ExportFileItem(url: url)
                            }
                        }
                    }
                    Button("导出 JSON 账单") {
                        Task {
                            if let url = await store.generateMonthlyExportFile(format: "json") {
                                exportItem = ExportFileItem(url: url)
                            }
                        }
                    }
                } label: {
                    Label("导出", systemImage: "square.and.arrow.up")
                        .font(.caption.weight(.medium))
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)

            HStack(spacing: 0) {
                monthlySummaryCell(
                    title: "月度等效费用",
                    value: TokenVaneFormatters.formatCurrency(mo?.apiEquivalentCostMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate)
                )
                monthlySummaryCell(
                    title: "月总 Tokens",
                    value: TokenVaneFormatters.formatCompactTokens(mo?.monthTotalTokens ?? 0)
                )
                monthlySummaryCell(
                    title: "缓存节省",
                    value: TokenVaneFormatters.formatCurrency(mo?.monthCacheSavedMicroUsd ?? 0, currency: store.currency, rateString: store.usdToCnyRate)
                )
            }
            .padding(.horizontal, 16)

            HairlineDivider(insetLeading: 16)

            VStack(alignment: .leading, spacing: 8) {
                Text("近 365 天活跃热力图")
                    .font(.subheadline.weight(.semibold))
                Heatmap365Canvas(cells: mo?.heatmap365d ?? [])
                    .frame(height: 84)
            }
            .padding(.horizontal, 16)

            HairlineDivider(insetLeading: 16)

            VStack(alignment: .leading, spacing: 8) {
                Text("按日明细账单")
                    .font(.subheadline.weight(.semibold))
                let rows = (mo?.dailyLedger ?? []).sorted { $0.date > $1.date }
                if rows.isEmpty {
                    Text("本月暂无按日账单记录")
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
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func flattenedPoints(from report: TrendsReportDTO?) -> [FlatTrendPoint] {
        guard let report else { return [] }
        var out: [FlatTrendPoint] = []
        for s in report.series.prefix(6) {
            let color = TokenVaneFormatters.providerColor(s.key)
            for p in s.points {
                out.append(FlatTrendPoint(
                    seriesKey: s.key,
                    seriesLabel: s.label,
                    date: Date(timeIntervalSince1970: Double(p.bucketStartMs) / 1000.0),
                    bucketMs: p.bucketStartMs,
                    tokens: Double(p.tokens),
                    costMicroUsd: p.costMicroUsd,
                    color: color
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

    private func formatBucketTimestamp(_ ms: Int64, granularity: TrendGranularityDTO) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000.0)
        let df = DateFormatter()
        df.dateFormat = (granularity == .daily || granularity == .weekly) ? "MM-dd" : "MM-dd HH:mm"
        return df.string(from: date)
    }

    private func formatDelta(_ delta: Double?) -> String {
        guard let delta, delta.isFinite else { return "—" }
        let prefix = delta > 0 ? "+" : ""
        return String(format: "%@%.1f%%", prefix, delta)
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

    var body: some View {
        let recent = Array(cells.suffix(364))
        let maxTokens = max(1, recent.map(\.totalTokens).max() ?? 1)
        Canvas { context, size in
            let cols = 52
            let rows = 7
            let spacing: CGFloat = 2
            let cellW = max(2, (size.width - CGFloat(cols - 1) * spacing) / CGFloat(cols))
            let cellH = max(2, (size.height - CGFloat(rows - 1) * spacing) / CGFloat(rows))
            let cellSide = min(cellW, cellH)

            for idx in 0..<(cols * rows) {
                let col = idx / rows
                let row = idx % rows
                let x = CGFloat(col) * (cellSide + spacing)
                let y = CGFloat(row) * (cellSide + spacing)
                let rect = CGRect(x: x, y: y, width: cellSide, height: cellSide)

                let dataIndex = idx - ((cols * rows) - recent.count)
                let tokens = (dataIndex >= 0 && dataIndex < recent.count) ? recent[dataIndex].totalTokens : 0
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
