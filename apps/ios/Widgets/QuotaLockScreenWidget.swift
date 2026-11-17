import SwiftUI
import WidgetKit

public struct QuotaLockScreenWidget: Widget {
    public let kind: String = "com.asecond.tokenvane.quota-lockscreen"

    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: DashboardTimelineProvider()) { entry in
            QuotaLockScreenEntryView(entry: entry)
                .containerBackground(.clear, for: .widget)
        }
        .configurationDisplayName("TokenVane 锁屏配额")
        .description("在锁屏一眼掌握今日花销、核心订阅窗口已用占比与重置倒计时。")
        .supportedFamilies([.accessoryCircular, .accessoryRectangular, .accessoryInline])
    }
}

private struct QuotaLockScreenEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: DashboardWidgetEntry

    var body: some View {
        let snap = entry.snapshot
        let primary = snap.primaryQuotas.first
        let cost = TokenVaneFormatters.formatCurrency(snap.todayCostMicroUsd, currency: snap.currency, rateString: snap.usdToCnyRate)
        let tokens = TokenVaneFormatters.formatCompactTokens(snap.todayTokens)

        switch family {
        case .accessoryCircular:
            let pct = min(100.0, max(0.0, primary?.usedPercent ?? 0.0))
            Gauge(value: pct, in: 0...100) {
                Text(primary.map { TokenVaneFormatters.providerShortCode($0.providerId) } ?? "TV")
            } currentValueLabel: {
                Text("\(Int(pct.rounded()))%")
                    .font(.caption2.weight(.bold))
                    .monospacedDigit()
            }
            .gaugeStyle(.accessoryCircularCapacity)

        case .accessoryRectangular:
            VStack(alignment: .leading, spacing: 3) {
                Text("今日 \(cost) · \(tokens)")
                    .font(.caption.weight(.semibold))
                    .monospacedDigit()
                    .lineLimit(1)
                if let primary {
                    ProgressView(value: min(1.0, max(0.0, primary.usedPercent / 100.0)))
                        .progressViewStyle(.linear)
                    Text("\(TokenVaneFormatters.providerShortCode(primary.providerId)) \(primary.windowLabel) \(TokenVaneFormatters.formatPercent(primary.usedPercent, decimals: 0)) · \(TokenVaneFormatters.formatCountdown(untilMs: primary.resetsAtMs, now: entry.date))")
                        .font(.caption2)
                        .monospacedDigit()
                        .lineLimit(1)
                } else {
                    Text("设备: \(snap.activeDeviceCount) · 等待配额")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

        default: // .accessoryInline
            if let primary {
                Text("TV \(cost) · \(TokenVaneFormatters.providerShortCode(primary.providerId)) \(Int(primary.usedPercent.rounded()))%")
                    .monospacedDigit()
            } else {
                Text("TV \(cost) · \(tokens)")
                    .monospacedDigit()
            }
        }
    }
}
