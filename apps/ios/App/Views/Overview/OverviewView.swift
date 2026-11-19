import Charts
import SwiftUI

private enum OverviewShareMetric: String, CaseIterable, Identifiable {
    case tokens
    case cost

    var id: String { rawValue }

    var title: String {
        switch self {
        case .tokens: return L10n.tr("overview.shares.unit", default: "Tokens")
        case .cost: return L10n.tr("overview.shares.unit_cost", default: "费用")
        }
    }
}

private struct IntradayPulseSegment: Identifiable {
    let bucketStartMs: Int64
    let date: Date
    let typeKey: String
    let typeLabel: String
    let tokens: Double

    var id: String { "\(bucketStartMs)::\(typeKey)" }
}

public struct OverviewView: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Bindable var store: AppStore
    @State private var shareMetric: OverviewShareMetric = .tokens
    @State private var showingSpeedPopover = false

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if let coreError = store.coreInitError {
                    coreErrorBanner(coreError)
                } else if !store.syncConfigConfigured && (store.bootstrap?.syncedDevices.isEmpty ?? true) {
                    unpairedEmptyBanner
                } else {
                    heroMetricsSection
                    HairlineDivider(insetLeading: 16)
                    intradayPulseSection
                    HairlineDivider(insetLeading: 16)
                    sharesBreakdownSection
                    HairlineDivider(insetLeading: 16)
                    syncedDevicesPulseSection
                    if !(store.overview?.providers.isEmpty ?? true) {
                        HairlineDivider(insetLeading: 16)
                        providerSummarySection
                    }
                    HairlineDivider(insetLeading: 16)
                    topQuotasSection
                }
            }
            .padding(.bottom, 24)
        }
        .refreshable {
            await store.triggerManualSync()
        }
        .background(Color.clear)
    }

    private var unpairedEmptyBanner: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L10n.tr("overview.empty.title", default: "连接同步中枢"))
                .font(.title3.weight(.semibold))
            Button {
                store.selectedTab = .settings
            } label: {
                Text(L10n.tr("overview.empty.action", default: "配置配对"))
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }
            .accessibilityIdentifier("overview.pairActionButton")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 28)
    }

    private func coreErrorBanner(_ message: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L10n.tr("overview.core_error.title", default: "数据库打开异常"))
                .font(.title3.weight(.semibold))
            Text(message)
                .font(.caption.monospaced())
                .foregroundStyle(SemanticColors.statusDanger)
                .textSelection(.enabled)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 28)
        .accessibilityIdentifier("overview.coreErrorBanner")
    }

    private var heroMetricsSection: some View {
        let ov = store.overview
        let rate = store.usdToCnyRate
        let cur = store.currency
        let liveTps = ov?.latestOutputTps
        let hasLiveSpeed = (liveTps ?? 0) > 0
        let speeds = ov?.liveModelSpeeds ?? []

        return VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(L10n.tr("overview.hero.today", default: "今日花费"))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)

                    Text(TokenVaneFormatters.formatCurrency(ov?.todayCostMicroUsd ?? 0, currency: cur, rateString: rate))
                        .font(.largeTitle.weight(.semibold))
                        .tabularMetrics()
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .accessibilityIdentifier("overview.totalCost")

                    DeltaBadge(percent: ov?.todayCostDeltaPct)
                }

                Spacer()

                VStack(alignment: .trailing, spacing: 4) {
                    Text(L10n.tr("overview.metric.tps_live", default: "实时速率"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Button {
                        showingSpeedPopover.toggle()
                    } label: {
                        HStack(spacing: 5) {
                            Circle()
                                .fill(hasLiveSpeed ? SemanticColors.statusLive : Color.secondary.opacity(0.4))
                                .frame(width: 7, height: 7)
                            Text(
                                hasLiveSpeed
                                    ? TokenVaneFormatters.formatTps(liveTps)
                                    : L10n.tr("overview.live_speed.idle", default: "空闲")
                            )
                            .font(.callout.weight(.semibold))
                            .tabularMetrics()
                            .lineLimit(1)
                            if !speeds.isEmpty {
                                Image(systemName: "chevron.down")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .popover(isPresented: $showingSpeedPopover) {
                        liveSpeedPopoverContent(speeds: speeds, ov: ov)
                    }

                    if let avg = ov?.avgOutputTps, avg > 0 {
                        Text(L10n.trFormat("overview.tps_avg_sub", default: "今日均 %@", TokenVaneFormatters.formatTps(avg)))
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                    }
                }
            }

            if dynamicTypeSize.isAccessibilitySize {
                LazyVGrid(
                    columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)],
                    alignment: .leading,
                    spacing: 12
                ) {
                    metricCells(ov: ov)
                }
            } else {
                LazyVGrid(
                    columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)],
                    alignment: .leading,
                    spacing: 12
                ) {
                    metricCells(ov: ov)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    @ViewBuilder
    private func liveSpeedPopoverContent(speeds: [LiveModelSpeedDTO], ov: OverviewReportDTO?) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(L10n.tr("overview.live_speed.popover_title", default: "近期生成速率 (3m 窗口)"))
                .font(.caption.weight(.semibold))
            if speeds.isEmpty {
                Text(L10n.tr("overview.live_speed.popover_empty", default: "近 3 分钟内暂无活跃流式输出"))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(speeds) { item in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(item.activityState == "active" ? SemanticColors.statusLive : SemanticColors.statusWarning)
                            .frame(width: 6, height: 6)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(item.modelId)
                                .font(.caption.weight(.medium))
                                .lineLimit(1)
                            Text(TokenVaneFormatters.providerDisplayName(item.providerId))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 12)
                        Text(TokenVaneFormatters.formatTps(item.outputTps))
                            .font(.caption.weight(.semibold))
                            .tabularMetrics()
                    }
                }
            }
            if let peak = ov?.peakOutputTps, peak > 0 {
                HairlineDivider()
                HStack {
                    Text(L10n.tr("overview.live_speed.peak", default: "今日峰值"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(TokenVaneFormatters.formatTps(peak))
                        .font(.caption2.weight(.semibold))
                        .tabularMetrics()
                }
            }
        }
        .padding(14)
        .frame(minWidth: 240)
        .presentationCompactAdaptation(.popover)
    }

    @ViewBuilder
    private func metricCells(ov: OverviewReportDTO?) -> some View {
        let rate = store.usdToCnyRate
        let cur = store.currency

        VStack(alignment: .leading, spacing: 3) {
            Text(L10n.tr("overview.metric.today_tokens", default: "今日 Tokens"))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text(TokenVaneFormatters.formatCompactTokens(ov?.todayTokens ?? 0))
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
            DeltaBadge(percent: ov?.todayTokensDeltaPct)
        }
        .frame(maxWidth: .infinity, alignment: .leading)

        VStack(alignment: .leading, spacing: 3) {
            Text(L10n.tr("overview.metric.cache_hit", default: "缓存命中"))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text(TokenVaneFormatters.formatPercent((ov?.todayCacheHitRatio ?? 0) * 100.0))
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
            Text(
                L10n.trFormat(
                    "overview.cache_saved_sub",
                    default: "已省 %@",
                    TokenVaneFormatters.formatCurrency(ov?.todayCacheSavedMicroUsd ?? 0, currency: cur, rateString: rate)
                )
            )
            .font(.caption2)
            .tabularMetrics()
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)

        VStack(alignment: .leading, spacing: 3) {
            Text(L10n.tr("overview.metric.requests", default: "请求数"))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text(TokenVaneFormatters.formatExactInt(ov?.todayRequests ?? 0))
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
            Text(
                L10n.trFormat(
                    "overview.peak_tps_sub",
                    default: "峰值 %@",
                    TokenVaneFormatters.formatTps(ov?.peakOutputTps)
                )
            )
            .font(.caption2)
            .tabularMetrics()
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)

        let activeProviders = (ov?.providers ?? []).filter { $0.todayTokens > 0 || $0.periodTokens > 0 }.count
        let devicesCount = store.bootstrap?.syncedDevices.count ?? 0
        VStack(alignment: .leading, spacing: 3) {
            Text(L10n.tr("overview.metric.channels", default: "活跃通道"))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text(L10n.trFormat("overview.providers_active", default: "%lld 服务商", Int64(activeProviders)))
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
            Text(L10n.trFormat("overview.devices.count", default: "%lld 台", Int64(devicesCount)))
                .font(.caption2)
                .tabularMetrics()
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var intradayPulseSection: some View {
        let pulsePoints = (store.overview?.todayHourlyPulse.isEmpty == false
            ? store.overview?.todayHourlyPulse
            : store.overview?.series) ?? []
        let segments = buildPulseSegments(from: pulsePoints)

        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(L10n.tr("overview.pulse.title", default: "今日实时脉冲 (24h)"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text(L10n.tr("overview.pulse.granularity", default: "按小时"))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            if segments.isEmpty {
                Text(L10n.tr("overview.pulse.waiting", default: "今日等待第一笔脉冲"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 12)
            } else {
                Chart(segments) { seg in
                    BarMark(
                        x: .value(L10n.tr("chart.axis.time", default: "时间"), seg.date, unit: .hour),
                        y: .value(L10n.tr("chart.axis.tokens", default: "Tokens"), seg.tokens)
                    )
                    .foregroundStyle(by: .value(L10n.tr("chart.axis.series", default: "类型"), seg.typeLabel))
                }
                .chartForegroundStyleScale([
                    "Input": TokenVaneFormatters.tokenTypeColor("input"),
                    "Output": TokenVaneFormatters.tokenTypeColor("output"),
                    "Cache Read": TokenVaneFormatters.tokenTypeColor("cache_read"),
                    "Cache Write": TokenVaneFormatters.tokenTypeColor("cache_write")
                ])
                .chartLegend(position: .bottom, alignment: .leading)
                .chartXAxis {
                    AxisMarks(values: .stride(by: .hour, count: 4)) { value in
                        AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5))
                            .foregroundStyle(SemanticColors.hairline)
                        AxisValueLabel {
                            if let d = value.as(Date.self) {
                                Text(TokenVaneFormatters.formatChartAxisDate(d, subDay: true))
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
                .frame(height: 150)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func buildPulseSegments(from points: [TimeBucketPointDTO]) -> [IntradayPulseSegment] {
        var out: [IntradayPulseSegment] = []
        for p in points {
            let date = Date(timeIntervalSince1970: Double(p.bucketStartMs) / 1000.0)
            if p.inputTokens > 0 {
                out.append(IntradayPulseSegment(bucketStartMs: p.bucketStartMs, date: date, typeKey: "input", typeLabel: "Input", tokens: Double(p.inputTokens)))
            }
            let effOut = p.outputTokens + p.reasoningTokens
            if effOut > 0 {
                out.append(IntradayPulseSegment(bucketStartMs: p.bucketStartMs, date: date, typeKey: "output", typeLabel: "Output", tokens: Double(effOut)))
            }
            if p.cacheReadTokens > 0 {
                out.append(IntradayPulseSegment(bucketStartMs: p.bucketStartMs, date: date, typeKey: "cache_read", typeLabel: "Cache Read", tokens: Double(p.cacheReadTokens)))
            }
            if p.cacheWriteTokens > 0 {
                out.append(IntradayPulseSegment(bucketStartMs: p.bucketStartMs, date: date, typeKey: "cache_write", typeLabel: "Cache Write", tokens: Double(p.cacheWriteTokens)))
            }
        }
        return out
    }

    private var sharesBreakdownSection: some View {
        let providers = store.overview?.providerShares ?? []
        let models = store.overview?.topModelShares ?? []
        let segments = providers.prefix(6).map {
            ShareBarSegment(
                id: $0.key,
                label: $0.label,
                sharePercent: shareMetric == .tokens ? $0.tokenSharePercent : $0.costSharePercent,
                color: TokenVaneFormatters.providerColor($0.key)
            )
        }

        return VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(L10n.tr("overview.shares.today", default: "今日分布"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Picker(L10n.tr("overview.shares.metric_picker", default: "指标"), selection: $shareMetric) {
                    ForEach(OverviewShareMetric.allCases) { m in
                        Text(m.title).tag(m)
                    }
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 140)
            }

            InlineShareBar(segments: segments, height: 6)

            if providers.isEmpty {
                Text(L10n.tr("overview.shares.waiting", default: "等待数据"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 4)
            } else {
                HStack(alignment: .top, spacing: 16) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(L10n.tr("overview.shares.by_provider", default: "服务商"))
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.secondary)
                        ForEach(providers.prefix(5)) { item in
                            let pct = shareMetric == .tokens ? item.tokenSharePercent : item.costSharePercent
                            HStack(spacing: 6) {
                                Circle()
                                    .fill(TokenVaneFormatters.providerColor(item.key))
                                    .frame(width: 7, height: 7)
                                Text(item.label)
                                    .font(.caption)
                                    .lineLimit(1)
                                Spacer(minLength: 4)
                                Text(TokenVaneFormatters.formatPercent(pct))
                                    .font(.caption)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)

                    VStack(alignment: .leading, spacing: 8) {
                        Text(L10n.tr("overview.shares.top_models", default: "Top 模型"))
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.secondary)
                        ForEach(models.prefix(5)) { item in
                            HStack(spacing: 6) {
                                Text(item.label)
                                    .font(.caption)
                                    .lineLimit(1)
                                Spacer(minLength: 4)
                                Text(
                                    shareMetric == .tokens
                                        ? TokenVaneFormatters.formatCompactTokens(item.tokens)
                                        : TokenVaneFormatters.formatCurrency(item.costMicroUsd, currency: store.currency, rateString: store.usdToCnyRate)
                                )
                                .font(.caption)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var syncedDevicesPulseSection: some View {
        let devices = store.bootstrap?.syncedDevices ?? []
        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(L10n.tr("overview.devices.title", default: "同步设备"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text(L10n.trFormat("overview.devices.count", default: "%lld 台", Int64(devices.count)))
                    .font(.caption2)
                    .tabularMetrics()
                    .foregroundStyle(.secondary)
            }

            if devices.isEmpty {
                Text(L10n.tr("overview.devices.waiting", default: "等待设备同步"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(devices) { dev in
                    HStack(spacing: 10) {
                        Circle()
                            .fill(dev.todayTokens > 0 ? SemanticColors.statusLive : Color.secondary.opacity(0.4))
                            .frame(width: 7, height: 7)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(dev.deviceName)
                                .font(.subheadline.weight(.medium))
                            Text(
                                L10n.trFormat(
                                    "overview.devices.last_active",
                                    default: "活跃 · %@",
                                    TokenVaneFormatters.formatRelativeSync(atMs: dev.lastActivityMs)
                                )
                            )
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                        }
                        Spacer()
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(TokenVaneFormatters.formatCurrency(dev.todayCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.subheadline.weight(.medium))
                                .tabularMetrics()
                            Text(
                                L10n.trFormat(
                                    "overview.devices.today_prefix",
                                    default: "今日 %@",
                                    TokenVaneFormatters.formatCompactTokens(dev.todayTokens)
                                )
                            )
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.vertical, 4)
                    if dev.id != devices.last?.id {
                        HairlineDivider()
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var providerSummarySection: some View {
        let rows = (store.overview?.providers ?? [])
            .filter { $0.todayTokens > 0 || $0.monthTokens > 0 }
        return VStack(alignment: .leading, spacing: 10) {
            Text(L10n.tr("overview.providers.title", default: "服务商花费"))
                .font(.subheadline.weight(.semibold))
            if rows.isEmpty {
                Text(L10n.tr("overview.shares.waiting", default: "等待数据"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(rows) { row in
                    HStack(spacing: 10) {
                        Circle()
                            .fill(TokenVaneFormatters.providerColor(row.providerId))
                            .frame(width: 7, height: 7)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(TokenVaneFormatters.providerDisplayName(row.providerId))
                                    .font(.subheadline.weight(.medium))
                                if let label = row.activeWindowLabel, let pct = row.activeWindowUsedPercent {
                                    Text("\(label) \(TokenVaneFormatters.formatPercent(pct, decimals: 0))")
                                        .font(.caption2)
                                        .tabularMetrics()
                                        .foregroundStyle(TokenVaneFormatters.quotaSeverityColor(usedPercent: pct))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1)
                                        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 3, style: .continuous))
                                }
                            }
                            Text("今日 \(TokenVaneFormatters.formatCompactTokens(row.todayTokens)) · 本月 \(TokenVaneFormatters.formatCompactTokens(row.monthTokens))")
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(TokenVaneFormatters.formatCurrency(row.todayCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.subheadline.weight(.medium))
                                .tabularMetrics()
                            Text("本月 \(TokenVaneFormatters.formatCurrency(row.monthCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.vertical, 4)
                    if row.id != rows.last?.id {
                        HairlineDivider()
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var topQuotasSection: some View {
        let sorted = (store.bootstrap?.remoteSubscriptions ?? [])
            .sorted { $0.usedPercent > $1.usedPercent }
        return VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(L10n.tr("overview.quotas.title", default: "订阅配额"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Button(L10n.tr("overview.quotas.all", default: "全部")) {
                    store.selectedTab = .quotas
                }
                .font(.caption)
            }

            if sorted.isEmpty {
                Text(L10n.tr("overview.quotas.waiting", default: "配额待同步"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(sorted.prefix(3)) { sub in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text("\(sub.planName) · \(sub.windowLabel)")
                                .font(.caption.weight(.medium))
                            Text("(\(sub.deviceDisplayName))")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(TokenVaneFormatters.formatPercent(sub.usedPercent))
                                .font(.caption.weight(.semibold))
                                .tabularMetrics()
                                .foregroundStyle(TokenVaneFormatters.quotaSeverityColor(usedPercent: sub.usedPercent))
                        }
                        LinearProgressStrip(
                            progressPercent: sub.usedPercent,
                            tint: TokenVaneFormatters.quotaSeverityColor(usedPercent: sub.usedPercent)
                        )
                        HStack {
                            Text(
                                sub.source == "official_header" || sub.source == "official_api" || sub.source == "official_cli" || sub.source == "official_log_snapshot"
                                    ? L10n.tr("overview.quotas.source.official", default: "官方")
                                    : L10n.tr("overview.quotas.source.estimated", default: "推算")
                            )
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            Spacer()
                            HStack(spacing: 0) {
                                Text(L10n.tr("overview.quotas.reset_prefix", default: "重置 · "))
                                QuotaCountdownText(resetsAtMs: sub.resetsAtMs)
                            }
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }
}
