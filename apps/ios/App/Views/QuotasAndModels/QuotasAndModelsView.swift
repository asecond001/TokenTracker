import SwiftUI
import UIKit

public struct QuotasAndModelsView: View {
    @Environment(\.openURL) private var openURL
    @Bindable var store: AppStore

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                subscriptionsSection
                HairlineDivider(insetLeading: 16)
                modelsAndSpeedSection
            }
            .padding(.bottom, 24)
        }
        .refreshable {
            await store.triggerManualSync()
        }
        .background(SemanticColors.surfacePrimary)
    }

    private var filteredSubscriptions: [RemoteSubscriptionSnapshotDTO] {
        let all = store.bootstrap?.remoteSubscriptions ?? []
        if let devId = store.selectedDeviceId {
            return all.filter { $0.deviceId == devId }
        }
        return all
    }

    private var groupedSubscriptions: [(deviceName: String, items: [RemoteSubscriptionSnapshotDTO])] {
        let grouped = Dictionary(grouping: filteredSubscriptions, by: \.deviceDisplayName)
        return grouped.keys.sorted().map { key in
            let sortedItems = (grouped[key] ?? []).sorted { $0.usedPercent > $1.usedPercent }
            return (deviceName: key, items: sortedItems)
        }
    }

    private var subscriptionsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(L10n.tr("quotas.section.title", default: "订阅窗口"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
            }

            if let feedback = store.liveActivityFeedback {
                HStack(spacing: 8) {
                    Text(feedback.message)
                        .font(.caption)
                        .foregroundStyle(SemanticColors.statusWarning)
                    Spacer()
                    if feedback.canOpenSettings,
                       let settingsURL = URL(string: UIApplication.openSettingsURLString) {
                        Button(L10n.tr("live_activity.open_settings", default: "系统设置")) {
                            openURL(settingsURL)
                        }
                        .font(.caption.weight(.semibold))
                    }
                    Button {
                        store.dismissLiveActivityFeedback()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }

            if groupedSubscriptions.isEmpty {
                Text(L10n.tr("quotas.waiting", default: "订阅窗口待同步"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 6)
            } else {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(groupedSubscriptions, id: \.deviceName) { group in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(group.deviceName)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            ForEach(group.items) { sub in
                                subscriptionRow(sub)
                                HairlineDivider()
                            }
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func subscriptionRow(_ sub: RemoteSubscriptionSnapshotDTO) -> some View {
        let isPinned = store.pinnedWatchKeys.contains(sub.id)
        let severityColor = TokenVaneFormatters.quotaSeverityColor(usedPercent: sub.usedPercent)

        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Circle()
                    .fill(TokenVaneFormatters.providerColor(sub.providerId))
                    .frame(width: 7, height: 7)
                Text("\(sub.planName) · \(sub.windowLabel)")
                    .font(.subheadline.weight(.medium))
                Spacer()
                Button {
                    Task { await store.toggleLiveActivityWatch(for: sub) }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: isPinned ? "stop.circle.fill" : "dot.radiowaves.left.and.right")
                            .font(.caption2)
                        Text(
                            isPinned
                                ? L10n.tr("quota.unpin_live_activity", default: "停止")
                                : L10n.tr("quota.pin_live_activity", default: "盯盘")
                        )
                        .font(.caption2.weight(.medium))
                    }
                    .foregroundStyle(isPinned ? SemanticColors.statusDanger : SemanticColors.accentIndigo)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .strokeBorder(
                                isPinned ? SemanticColors.statusDanger.opacity(0.4) : SemanticColors.accentIndigo.opacity(0.4),
                                lineWidth: 1
                            )
                    )
                }
                .buttonStyle(.plain)
            }

            HStack {
                LinearProgressStrip(progressPercent: sub.usedPercent, tint: severityColor)
                Text(TokenVaneFormatters.formatPercent(sub.usedPercent))
                    .font(.caption.weight(.semibold))
                    .tabularMetrics()
                    .foregroundStyle(severityColor)
                    .frame(width: 52, alignment: .trailing)
            }

            HStack {
                Text(
                    sub.source == "official_header"
                        ? L10n.tr("overview.quotas.source.official", default: "官方")
                        : L10n.tr("overview.quotas.source.estimated", default: "推算")
                )
                .font(.caption2)
                .foregroundStyle(.secondary)
                Spacer()
                HStack(spacing: 0) {
                    Text(L10n.tr("quota.reset_countdown_prefix", default: "重置 "))
                    QuotaCountdownText(resetsAtMs: sub.resetsAtMs)
                }
                .font(.caption2)
                .tabularMetrics()
                .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
    }

    private var modelsAndSpeedSection: some View {
        let rows = store.modelsReport?.rows ?? []
        let speedMap = Dictionary(
            (store.speedReport?.models ?? []).map { ($0.id, $0) },
            uniquingKeysWith: { first, _ in first }
        )
        let fromRollups = store.speedReport?.fromHourlyRollups ?? false

        return VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text(L10n.tr("models.section.title", default: "模型效率"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                if fromRollups {
                    Text(L10n.tr("models.rollups_badge", default: "小时聚合"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .overlay(
                            RoundedRectangle(cornerRadius: 3, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }
            }

            if rows.isEmpty {
                Text(L10n.tr("models.waiting", default: "等待模型数据"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 8)
            } else {
                ForEach(rows) { row in
                    let speed = speedMap[row.id]
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(row.modelId)
                                .font(.subheadline.weight(.medium))
                                .lineLimit(1)
                            Text(TokenVaneFormatters.providerShortCode(row.providerId))
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(TokenVaneFormatters.providerColor(row.providerId))
                            Spacer()
                            Text(TokenVaneFormatters.formatCurrency(row.estimatedCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                                .font(.subheadline.weight(.semibold))
                                .tabularMetrics()
                        }

                        HStack(spacing: 12) {
                            Text("Token \(TokenVaneFormatters.formatCompactTokens(row.totalTokens))")
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                            Text(
                                L10n.trFormat(
                                    "models.cache_prefix",
                                    default: "缓存 %@",
                                    TokenVaneFormatters.formatPercent(row.cacheHitPercent, decimals: 0)
                                )
                            )
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                            Spacer()
                            if let speed {
                                let label = fromRollups
                                    ? L10n.trFormat(
                                        "models.tps_rollup",
                                        default: "均 %@ · 峰 %@",
                                        TokenVaneFormatters.formatTps(speed.meanTps),
                                        String(format: "%.1f", speed.maxTps)
                                    )
                                    : "\(TokenVaneFormatters.formatTps(speed.meanTps)) · \(TokenVaneFormatters.formatDurationMs(speed.avgDurationMs))"
                                Text(label)
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            } else if row.avgOutputTps > 0 {
                                Text(TokenVaneFormatters.formatTps(row.avgOutputTps))
                                    .font(.caption2)
                                    .tabularMetrics()
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .padding(.vertical, 5)
                    HairlineDivider()
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }
}
