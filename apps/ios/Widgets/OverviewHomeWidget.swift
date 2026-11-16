import AppIntents
import SwiftUI
import WidgetKit

public struct DashboardWidgetEntry: TimelineEntry {
    public let date: Date
    public let snapshot: WidgetDashboardSnapshot
    public let isPlaceholder: Bool
}

public struct DashboardTimelineProvider: TimelineProvider {
    public func placeholder(in context: Context) -> DashboardWidgetEntry {
        DashboardWidgetEntry(date: Date(), snapshot: .placeholder, isPlaceholder: true)
    }

    public func getSnapshot(in context: Context, completion: @escaping (DashboardWidgetEntry) -> Void) {
        let loaded = AppGroupPaths.loadWidgetSnapshot()
        completion(DashboardWidgetEntry(
            date: Date(),
            snapshot: loaded ?? .placeholder,
            isPlaceholder: loaded == nil
        ))
    }

    public func getTimeline(in context: Context, completion: @escaping (Timeline<DashboardWidgetEntry>) -> Void) {
        let now = Date()
        let loaded = AppGroupPaths.loadWidgetSnapshot()
        let entry = DashboardWidgetEntry(
            date: now,
            snapshot: loaded ?? .placeholder,
            isPlaceholder: loaded == nil
        )
        let nextRefresh = Calendar.current.date(byAdding: .minute, value: 15, to: now) ?? now.addingTimeInterval(900)
        completion(Timeline(entries: [entry], policy: .after(nextRefresh)))
    }
}

public struct OverviewHomeWidget: Widget {
    public let kind: String = "com.tokentracker.app.overview-home"

    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: DashboardTimelineProvider()) { entry in
            OverviewHomeWidgetEntryView(entry: entry)
                .containerBackground(.background, for: .widget)
        }
        .configurationDisplayName("TokenTracker 今日总览")
        .description("实时查看今日 AI 花费、Provider 分布与核心订阅滚动窗口配额。")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

private struct OverviewHomeWidgetEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: DashboardWidgetEntry

    var body: some View {
        switch family {
        case .systemSmall:
            smallView
        default:
            mediumView
        }
    }

    private var smallView: some View {
        let snap = entry.snapshot
        let cost = TokenTrackerFormatters.formatCurrency(snap.todayCostMicroUsd, currency: snap.currency, rateString: snap.usdToCnyRate)
        let tokens = TokenTrackerFormatters.formatCompactTokens(snap.todayTokens)

        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("今日花费")
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                Button(intent: RefreshWidgetIntent()) {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }

            Text(cost)
                .font(.title2.weight(.bold))
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.75)

            Text("\(tokens) tokens")
                .font(.caption2)
                .monospacedDigit()
                .foregroundStyle(.secondary)

            widgetProviderBar(shares: snap.topProviders)

            Spacer(minLength: 2)

            if let quota = snap.primaryQuotas.first {
                VStack(alignment: .leading, spacing: 3) {
                    HStack {
                        Text("\(TokenTrackerFormatters.providerShortCode(quota.providerId)) \(quota.windowLabel)")
                            .font(.caption2.weight(.medium))
                            .lineLimit(1)
                        Spacer()
                        Text(TokenTrackerFormatters.formatPercent(quota.usedPercent, decimals: 0))
                            .font(.caption2.weight(.semibold))
                            .monospacedDigit()
                            .foregroundStyle(TokenTrackerFormatters.quotaSeverityColor(usedPercent: quota.usedPercent))
                    }
                    widgetBar(percent: quota.usedPercent)
                    Text(TokenTrackerFormatters.formatCountdown(untilMs: quota.resetsAtMs, now: entry.date))
                        .font(.system(size: 10))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
            } else {
                Text(entry.isPlaceholder ? "等待同步桌面数据" : "暂无订阅窗口")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var mediumView: some View {
        let snap = entry.snapshot
        let cost = TokenTrackerFormatters.formatCurrency(snap.todayCostMicroUsd, currency: snap.currency, rateString: snap.usdToCnyRate)

        return HStack(spacing: 12) {
            // Left column: Cost + Top 3 Providers
            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text("今日总览")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(TokenTrackerFormatters.formatTps(snap.latestOutputTps))
                        .font(.caption2)
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                Text(cost)
                    .font(.title2.weight(.bold))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)
                Text("\(TokenTrackerFormatters.formatCompactTokens(snap.todayTokens)) tok")
                    .font(.caption2)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)

                widgetProviderBar(shares: snap.topProviders)

                ForEach(snap.topProviders.prefix(3)) { item in
                    HStack(spacing: 4) {
                        Circle()
                            .fill(TokenTrackerFormatters.providerColor(item.providerId))
                            .frame(width: 6, height: 6)
                        Text(item.displayName)
                            .font(.system(size: 10))
                            .lineLimit(1)
                        Spacer()
                        Text(TokenTrackerFormatters.formatPercent(item.sharePercent, decimals: 0))
                            .font(.system(size: 10))
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Rectangle()
                .fill(Color.primary.opacity(0.1))
                .frame(width: 1)

            // Right column: Top subscription windows
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text("订阅窗口")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button(intent: RefreshWidgetIntent()) {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                }

                if snap.primaryQuotas.isEmpty {
                    Text("暂无远端订阅配额")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(snap.primaryQuotas.prefix(3)) { quota in
                        VStack(alignment: .leading, spacing: 2) {
                            HStack {
                                Text("\(TokenTrackerFormatters.providerShortCode(quota.providerId))·\(quota.windowLabel)")
                                    .font(.system(size: 11, weight: .medium))
                                    .lineLimit(1)
                                Spacer()
                                Text(TokenTrackerFormatters.formatPercent(quota.usedPercent, decimals: 0))
                                    .font(.system(size: 11, weight: .semibold))
                                    .monospacedDigit()
                                    .foregroundStyle(TokenTrackerFormatters.quotaSeverityColor(usedPercent: quota.usedPercent))
                            }
                            widgetBar(percent: quota.usedPercent)
                        }
                    }
                }
                Spacer(minLength: 0)
                Text("同步 · \(TokenTrackerFormatters.formatRelativeSync(atMs: snap.lastSyncAtMs, now: entry.date))")
                    .font(.system(size: 9.5))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func widgetProviderBar(shares: [WidgetProviderShare]) -> some View {
        GeometryReader { geo in
            if shares.isEmpty {
                Capsule()
                    .fill(Color.primary.opacity(0.1))
            } else {
                HStack(spacing: 1.5) {
                    ForEach(shares.prefix(4)) { share in
                        let w = max(3, geo.size.width * max(0, share.sharePercent / 100.0))
                        Rectangle()
                            .fill(TokenTrackerFormatters.providerColor(share.providerId))
                            .frame(width: w)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 2, style: .continuous))
            }
        }
        .frame(height: 5)
    }

    private func widgetBar(percent: Double) -> some View {
        GeometryReader { geo in
            let ratio = min(1.0, max(0.0, percent / 100.0))
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                    .fill(Color.primary.opacity(0.12))
                RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                    .fill(TokenTrackerFormatters.quotaSeverityColor(usedPercent: percent))
                    .frame(width: max(ratio > 0 ? 3 : 0, geo.size.width * ratio))
            }
        }
        .frame(height: 4)
    }
}
