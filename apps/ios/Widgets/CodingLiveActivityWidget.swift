import ActivityKit
import SwiftUI
import WidgetKit

public struct CodingLiveActivityWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        ActivityConfiguration(for: CodingSessionAttributes.self) { context in
            lockScreenBanner(context: context)
                .padding(14)
                .activityBackgroundTint(Color(red: 0.06, green: 0.07, blue: 0.09))
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            let now = Date()
            let isStale = isActivityStale(context: context, now: now)
            let severityColor = isStale
                ? Color.secondary
                : TokenVaneFormatters.quotaSeverityColor(usedPercent: context.state.usedPercent)

            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(context.attributes.providerName)
                            .font(.caption.weight(.semibold))
                            .lineLimit(1)
                        Text("\(context.attributes.deviceName) · \(context.attributes.windowLabel)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                DynamicIslandExpandedRegion(.trailing) {
                    VStack(alignment: .trailing, spacing: 2) {
                        if isStale {
                            Text(L10n.tr("live.stale", default: "待刷新"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                        } else {
                            Text(TokenVaneFormatters.formatPercent(context.state.usedPercent, decimals: 1))
                                .font(.headline.weight(.bold))
                                .monospacedDigit()
                                .foregroundStyle(severityColor)
                        }

                        if let resetDate = context.state.resetsAtDate, resetDate > now {
                            Text(timerInterval: now...resetDate, countsDown: true)
                                .font(.caption2)
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                        } else {
                            Text(L10n.tr("quota.resetting_soon", default: "即将重置"))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 6) {
                        ProgressView(value: isStale ? 0.0 : min(1.0, max(0.0, context.state.usedPercent / 100.0)))
                            .tint(severityColor)
                            .opacity(isStale ? 0.45 : 1.0)
                        HStack {
                            Text(L10n.trFormat("live.today_prefix", default: "今日 %@", context.state.todayCostFormatted))
                                .font(.caption2.weight(.medium))
                                .monospacedDigit()
                            Spacer()
                            Text(TokenVaneFormatters.formatTps(context.state.latestTps))
                                .font(.caption2)
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            } compactLeading: {
                Text("\(TokenVaneFormatters.providerShortCode(context.attributes.providerId)) \(context.attributes.windowId)")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(TokenVaneFormatters.providerColor(context.attributes.providerId))
            } compactTrailing: {
                if isStale {
                    Text(L10n.tr("live.stale", default: "待刷新"))
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.secondary)
                } else {
                    Text("\(Int(context.state.usedPercent.rounded()))%")
                        .font(.caption2.weight(.bold))
                        .monospacedDigit()
                        .foregroundStyle(severityColor)
                }
            } minimal: {
                let pct = isStale ? 0.0 : min(100.0, max(0.0, context.state.usedPercent))
                Gauge(value: pct, in: 0...100) {
                    Text(TokenVaneFormatters.providerShortCode(context.attributes.providerId))
                }
                .gaugeStyle(.accessoryCircularCapacity)
                .tint(severityColor)
            }
        }
    }

    private func isActivityStale(context: ActivityViewContext<CodingSessionAttributes>, now: Date) -> Bool {
        if context.isStale {
            return true
        }
        if let resetDate = context.state.resetsAtDate, resetDate <= now {
            return true
        }
        return false
    }

    private func lockScreenBanner(context: ActivityViewContext<CodingSessionAttributes>) -> some View {
        let now = Date()
        let isStale = isActivityStale(context: context, now: now)
        let severityColor = isStale
            ? Color.white.opacity(0.55)
            : TokenVaneFormatters.quotaSeverityColor(usedPercent: context.state.usedPercent)

        return VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(context.attributes.planName) · \(context.attributes.windowLabel)")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                    Text(context.attributes.deviceName)
                        .font(.caption2)
                        .foregroundStyle(.white.opacity(0.7))
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    if isStale {
                        Text(L10n.tr("live.stale", default: "待刷新"))
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white.opacity(0.65))
                    } else {
                        Text(TokenVaneFormatters.formatPercent(context.state.usedPercent, decimals: 1))
                            .font(.title3.weight(.bold))
                            .monospacedDigit()
                            .foregroundStyle(severityColor)
                    }
                    if let resetDate = context.state.resetsAtDate, resetDate > now {
                        HStack(spacing: 4) {
                            Text(L10n.tr("live.reset_label", default: "重置"))
                            Text(timerInterval: now...resetDate, countsDown: true)
                                .monospacedDigit()
                        }
                        .font(.caption2)
                        .foregroundStyle(.white.opacity(0.75))
                    } else {
                        Text(L10n.tr("quota.resetting_soon", default: "即将重置"))
                            .font(.caption2)
                            .foregroundStyle(.white.opacity(0.65))
                    }
                }
            }

            ProgressView(value: isStale ? 0.0 : min(1.0, max(0.0, context.state.usedPercent / 100.0)))
                .tint(severityColor)
                .opacity(isStale ? 0.45 : 1.0)

            HStack {
                Text(L10n.trFormat("live.today_total_prefix", default: "今日累计 %@", context.state.todayCostFormatted))
                    .font(.caption2)
                    .monospacedDigit()
                    .foregroundStyle(.white.opacity(0.85))
                Spacer()
                Text(L10n.trFormat("live.speed_prefix", default: "速率 %@", TokenVaneFormatters.formatTps(context.state.latestTps)))
                    .font(.caption2)
                    .monospacedDigit()
                    .foregroundStyle(.white.opacity(0.75))
            }
        }
    }
}
