import SwiftUI

public struct OverviewView: View {
    @Bindable var store: AppStore

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
                    sharesBreakdownSection
                    HairlineDivider(insetLeading: 16)
                    syncedDevicesPulseSection
                    HairlineDivider(insetLeading: 16)
                    topQuotasSection
                }
            }
            .padding(.bottom, 24)
        }
        .refreshable {
            await store.triggerManualSync()
        }
        .background(SemanticColors.surfacePrimary)
    }

    private var unpairedEmptyBanner: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("连接自托管云端中枢")
                .font(.title3.weight(.semibold))
            Button {
                store.selectedTab = .settings
            } label: {
                Text("前往扫码或配置配对")
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
            Text("本地数据库打开失败")
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

        return VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(store.timeRange == .today ? "今日实时态势" : "所选周期累计等效花费")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                    Text(TokenVaneFormatters.formatCurrency(ov?.totalCostMicroUsd ?? 0, currency: cur, rateString: rate))
                        .font(.system(size: 32, weight: .semibold))
                        .tabularMetrics()
                        .accessibilityIdentifier("overview.totalCost")
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 4) {
                    Text("今日花费")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(TokenVaneFormatters.formatCurrency(ov?.todayCostMicroUsd ?? 0, currency: cur, rateString: rate))
                        .font(.callout.weight(.semibold))
                        .tabularMetrics()
                }
            }

            HStack(spacing: 0) {
                metricCell(
                    title: "总 Token",
                    value: TokenVaneFormatters.formatCompactTokens(ov?.totalTokens ?? 0)
                )
                metricCell(
                    title: "缓存命中率",
                    value: TokenVaneFormatters.formatPercent((ov?.cacheHitRatio ?? 0) * 100.0)
                )
                metricCell(
                    title: "最新速率",
                    value: TokenVaneFormatters.formatTps(ov?.latestOutputTps)
                )
                metricCell(
                    title: "有效请求",
                    value: TokenVaneFormatters.formatExactInt(ov?.totalRequests ?? 0)
                )
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func metricCell(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.subheadline.weight(.semibold))
                .tabularMetrics()
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var sharesBreakdownSection: some View {
        let providers = store.overview?.providerShares ?? []
        let models = store.overview?.topModelShares ?? []
        let segments = providers.prefix(6).map {
            ShareBarSegment(
                id: $0.key,
                label: $0.label,
                sharePercent: $0.tokenSharePercent,
                color: TokenVaneFormatters.providerColor($0.key)
            )
        }

        return VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(store.timeRange == .today ? "今日用量分布" : "用量分布")
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text("Tokens 占比")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            InlineShareBar(segments: segments, height: 6)

            if providers.isEmpty {
                Text("等待用量数据")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 4)
            } else {
                HStack(alignment: .top, spacing: 16) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("按 Provider")
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.secondary)
                        ForEach(providers.prefix(4)) { item in
                            HStack(spacing: 6) {
                                Circle()
                                    .fill(TokenVaneFormatters.providerColor(item.key))
                                    .frame(width: 7, height: 7)
                                Text(item.label)
                                    .font(.caption)
                                    .lineLimit(1)
                                Spacer(minLength: 4)
                                Text(TokenVaneFormatters.formatPercent(item.tokenSharePercent))
                                    .font(.caption)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Top 模型")
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.secondary)
                        ForEach(models.prefix(4)) { item in
                            HStack(spacing: 6) {
                                Text(item.label)
                                    .font(.caption)
                                    .lineLimit(1)
                                Spacer(minLength: 4)
                                Text(TokenVaneFormatters.formatCompactTokens(item.tokens))
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
                Text("已同步桌面终端")
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text("\(devices.count) 台")
                    .font(.caption2)
                    .tabularMetrics()
                    .foregroundStyle(.secondary)
            }

            if devices.isEmpty {
                Text("等待桌面端首次同步")
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
                            Text("最近活跃 · \(TokenVaneFormatters.formatRelativeSync(atMs: dev.lastActivityMs))")
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(TokenVaneFormatters.formatCurrency(dev.todayCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.subheadline.weight(.medium))
                                .tabularMetrics()
                            Text("今日 \(TokenVaneFormatters.formatCompactTokens(dev.todayTokens))")
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

    private var topQuotasSection: some View {
        let sorted = (store.bootstrap?.remoteSubscriptions ?? [])
            .sorted { $0.usedPercent > $1.usedPercent }
        return VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("核心订阅窗口速览")
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Button("全部") {
                    store.selectedTab = .quotas
                }
                .font(.caption)
            }

            if sorted.isEmpty {
                Text("订阅窗口待同步")
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
                            Text(sub.source == "official_header" ? "官方接口" : "本地推算")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text("重置 · \(TokenVaneFormatters.formatCountdown(untilMs: sub.resetsAtMs))")
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
