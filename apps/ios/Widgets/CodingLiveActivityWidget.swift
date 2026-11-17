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
            let severityColor = TokenVaneFormatters.quotaSeverityColor(usedPercent: context.state.usedPercent)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(context.attributes.providerName)")
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
                        Text(TokenVaneFormatters.formatPercent(context.state.usedPercent, decimals: 1))
                            .font(.headline.weight(.bold))
                            .monospacedDigit()
                            .foregroundStyle(severityColor)
                        if let resetDate = context.state.resetsAtDate, resetDate > Date() {
                            Text(timerInterval: Date()...resetDate, countsDown: true)
                                .font(.caption2)
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                        } else {
                            Text("即将重置")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 6) {
                        ProgressView(value: min(1.0, max(0.0, context.state.usedPercent / 100.0)))
                            .tint(severityColor)
                        HStack {
                            Text("今日 \(context.state.todayCostFormatted)")
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
                Text("\(Int(context.state.usedPercent.rounded()))%")
                    .font(.caption2.weight(.bold))
                    .monospacedDigit()
                    .foregroundStyle(severityColor)
            } minimal: {
                let pct = min(100.0, max(0.0, context.state.usedPercent))
                Gauge(value: pct, in: 0...100) {
                    Text(TokenVaneFormatters.providerShortCode(context.attributes.providerId))
                }
                .gaugeStyle(.accessoryCircularCapacity)
                .tint(severityColor)
            }
        }
    }

    private func lockScreenBanner(context: ActivityViewContext<CodingSessionAttributes>) -> some View {
        let severityColor = TokenVaneFormatters.quotaSeverityColor(usedPercent: context.state.usedPercent)
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
                    Text(TokenVaneFormatters.formatPercent(context.state.usedPercent, decimals: 1))
                        .font(.title3.weight(.bold))
                        .monospacedDigit()
                        .foregroundStyle(severityColor)
                    if let resetDate = context.state.resetsAtDate, resetDate > Date() {
                        HStack(spacing: 4) {
                            Text("重置")
                            Text(timerInterval: Date()...resetDate, countsDown: true)
                                .monospacedDigit()
                        }
                        .font(.caption2)
                        .foregroundStyle(.white.opacity(0.75))
                    }
                }
            }

            ProgressView(value: min(1.0, max(0.0, context.state.usedPercent / 100.0)))
                .tint(severityColor)

            HStack {
                Text("今日累计 \(context.state.todayCostFormatted)")
                    .font(.caption2)
                    .monospacedDigit()
                    .foregroundStyle(.white.opacity(0.85))
                Spacer()
                Text("速率 \(TokenVaneFormatters.formatTps(context.state.latestTps))")
                    .font(.caption2)
                    .monospacedDigit()
                    .foregroundStyle(.white.opacity(0.75))
            }
        }
    }
}
