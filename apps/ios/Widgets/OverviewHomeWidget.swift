import AppIntents
import SwiftUI
import WidgetKit

public struct DashboardWidgetEntry: TimelineEntry, Sendable {
    public let date: Date
    public let snapshot: WidgetDashboardSnapshot
    public let isPlaceholder: Bool
}

public struct DashboardTimelineProvider: TimelineProvider {
    public func placeholder(in context: Context) -> DashboardWidgetEntry {
        let now = Date()
        return DashboardWidgetEntry(date: now, snapshot: .placeholder.effectiveForDisplay(at: now), isPlaceholder: true)
    }

    public func getSnapshot(in context: Context, completion: @escaping (DashboardWidgetEntry) -> Void) {
        let now = Date()
        let loaded = AppGroupPaths.loadWidgetSnapshot()
        let effective = (loaded ?? .placeholder).effectiveForDisplay(at: now)
        completion(DashboardWidgetEntry(
            date: now,
            snapshot: effective,
            isPlaceholder: loaded == nil
        ))
    }

    public func getTimeline(in context: Context, completion: @escaping (Timeline<DashboardWidgetEntry>) -> Void) {
        let now = Date()
        let cal = Calendar.current
        let loaded = AppGroupPaths.loadWidgetSnapshot()
        let base = loaded ?? .placeholder
        let isPlaceholder = (loaded == nil)

        let nextRefresh = cal.date(byAdding: .minute, value: 15, to: now) ?? now.addingTimeInterval(900)
        let nextMidnight = cal.nextDate(
            after: now,
            matching: DateComponents(hour: 0, minute: 0, second: 0),
            matchingPolicy: .nextTime
        ) ?? now.addingTimeInterval(86_400)

        var entryDates: [Date] = [now, nextMidnight]
        for quota in base.primaryQuotas {
            if let resetsAtMs = quota.resetsAtMs {
                let resetDate = Date(timeIntervalSince1970: Double(resetsAtMs) / 1000.0)
                if resetDate > now && resetDate <= nextMidnight {
                    entryDates.append(resetDate)
                }
            }
        }

        // Deduplicate closely spaced timestamps (< 1s) and sort chronologically.
        let sortedDates = entryDates.sorted().reduce(into: [Date]()) { acc, candidate in
            if let last = acc.last, candidate.timeIntervalSince(last) < 1.0 {
                return
            }
            acc.append(candidate)
        }

        let entries = sortedDates.map { date in
            DashboardWidgetEntry(
                date: date,
                snapshot: base.effectiveForDisplay(at: date, calendar: cal),
                isPlaceholder: isPlaceholder
            )
        }
        completion(Timeline(entries: entries, policy: .after(nextRefresh)))
    }
}

public struct OverviewHomeWidget: Widget {
    public let kind: String = "com.asecond.tokenvane.overview-home"

    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: DashboardTimelineProvider()) { entry in
            OverviewHomeWidgetEntryView(entry: entry)
                .containerBackground(.background, for: .widget)
        }
        .configurationDisplayName(LocalizedStringKey("widget.overview.title"))
        .description(LocalizedStringKey("widget.overview.desc"))
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
        let cost = TokenVaneFormatters.formatCurrency(snap.todayCostMicroUsd, currency: snap.currency, rateString: snap.usdToCnyRate)
        let tokens = TokenVaneFormatters.formatCompactTokens(snap.todayTokens)

        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(L10n.tr("widget.today_cost", default: "今日花费"))
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
                        Text("\(TokenVaneFormatters.providerShortCode(quota.providerId)) \(quota.windowLabel)")
                            .font(.caption2.weight(.medium))
                            .lineLimit(1)
                        Spacer()
                        Text(TokenVaneFormatters.formatPercent(quota.usedPercent, decimals: 0))
                            .font(.caption2.weight(.semibold))
                            .monospacedDigit()
                            .foregroundStyle(TokenVaneFormatters.quotaSeverityColor(usedPercent: quota.usedPercent))
                    }
                    widgetBar(percent: quota.usedPercent)
                    QuotaCountdownText(resetsAtMs: quota.resetsAtMs, referenceDate: entry.date)
                        .font(.system(size: 10))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
            } else {
                Text(
                    entry.isPlaceholder
                        ? L10n.tr("widget.waiting_sync", default: "等待同步")
                        : L10n.tr("widget.quotas_pending", default: "配额待同步")
                )
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
        }
    }

    private var mediumView: some View {
        let snap = entry.snapshot
        let cost = TokenVaneFormatters.formatCurrency(snap.todayCostMicroUsd, currency: snap.currency, rateString: snap.usdToCnyRate)

        return HStack(spacing: 12) {
            // Left column: Cost + Top 3 Providers
            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text(L10n.tr("widget.today_overview", default: "今日总览"))
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(TokenVaneFormatters.formatTps(snap.latestOutputTps))
                        .font(.caption2)
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                Text(cost)
                    .font(.title2.weight(.bold))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)
                Text("\(TokenVaneFormatters.formatCompactTokens(snap.todayTokens)) tok")
                    .font(.caption2)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)

                widgetProviderBar(shares: snap.topProviders)

                ForEach(snap.topProviders.prefix(3)) { item in
                    HStack(spacing: 4) {
                        Circle()
                            .fill(TokenVaneFormatters.providerColor(item.providerId))
                            .frame(width: 6, height: 6)
                        Text(item.displayName)
                            .font(.system(size: 10))
                            .lineLimit(1)
                        Spacer()
                        Text(TokenVaneFormatters.formatPercent(item.sharePercent, decimals: 0))
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
                    Text(L10n.tr("widget.quotas", default: "订阅窗口"))
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
                    Text(L10n.tr("widget.quotas_pending", default: "配额待同步"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(snap.primaryQuotas.prefix(3)) { quota in
                        VStack(alignment: .leading, spacing: 2) {
                            HStack {
                                Text("\(TokenVaneFormatters.providerShortCode(quota.providerId))·\(quota.windowLabel)")
                                    .font(.system(size: 11, weight: .medium))
                                    .lineLimit(1)
                                Spacer()
                                Text(TokenVaneFormatters.formatPercent(quota.usedPercent, decimals: 0))
                                    .font(.system(size: 11, weight: .semibold))
                                    .monospacedDigit()
                                    .foregroundStyle(TokenVaneFormatters.quotaSeverityColor(usedPercent: quota.usedPercent))
                            }
                            widgetBar(percent: quota.usedPercent)
                        }
                    }
                }
                Spacer(minLength: 0)
                Text(
                    L10n.trFormat(
                        "widget.sync_prefix",
                        default: "同步 · %@",
                        TokenVaneFormatters.formatRelativeSync(atMs: snap.lastSyncAtMs, now: entry.date)
                    )
                )
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
                            .fill(TokenVaneFormatters.providerColor(share.providerId))
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
                    .fill(TokenVaneFormatters.quotaSeverityColor(usedPercent: percent))
                    .frame(width: max(ratio > 0 ? 3 : 0, geo.size.width * ratio))
            }
        }
        .frame(height: 4)
    }
}
