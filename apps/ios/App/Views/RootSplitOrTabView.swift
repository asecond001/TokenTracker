import SwiftUI

public struct RootSplitOrTabView: View {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Bindable var store: AppStore

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        Group {
            if sizeClass == .regular {
                iPadLayout
            } else {
                iPhoneLayout
            }
        }
        .background(SemanticColors.surfacePrimary.ignoresSafeArea())
        .tint(SemanticColors.accentIndigo)
        .alert(
            L10n.tr("pairing.alert.title", default: "连接同步中枢"),
            isPresented: Binding(
                get: { store.pendingPairing != nil },
                set: { if !$0 { store.cancelPendingPairing() } }
            ),
            presenting: store.pendingPairing
        ) { payload in
            // Capture the payload: the binding clears `pendingPairing` as the alert dismisses.
            Button(L10n.tr("pairing.alert.connect", default: "连接")) {
                Task { await store.confirmPairing(payload) }
            }
            .accessibilityIdentifier("pairing.confirmButton")
            Button(L10n.tr("pairing.alert.cancel", default: "取消"), role: .cancel) {
                store.cancelPendingPairing()
            }
        } message: { payload in
            Text(pairingSummary(payload))
        }
    }

    private func pairingSummary(_ payload: DesktopPairingPayload) -> String {
        let host = URL(string: payload.url)?.host ?? payload.url
        let mode = payload.e2ee
            ? L10n.tr("pairing.summary.mode.e2ee", default: "端到端加密")
            : L10n.tr("pairing.summary.mode.plain", default: "明文聚合")
        var lines = [
            L10n.trFormat("pairing.summary.worker", default: "Worker · %@", host),
            L10n.trFormat("pairing.summary.mode", default: "传输 · %@", mode)
        ]
        if store.syncConfigConfigured,
           let current = store.bootstrap?.syncConfig.workerUrl,
           URL(string: current)?.host != URL(string: payload.url)?.host {
            lines.append(
                L10n.trFormat("pairing.summary.replace", default: "将替换 · %@", URL(string: current)?.host ?? current)
            )
        }
        return lines.joined(separator: "\n")
    }

    private var iPhoneLayout: some View {
        TabView(selection: $store.selectedTab) {
            NavigationStack {
                VStack(spacing: 0) {
                    globalContextStrip
                    HairlineDivider()
                    resetAlertBanner
                    OverviewView(store: store)
                }
                .background(SemanticColors.surfacePrimary)
                .navigationTitle(MainTab.overview.navTitle)
                .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label(MainTab.overview.title, systemImage: MainTab.overview.systemImage)
            }
            .tag(MainTab.overview)

            NavigationStack {
                VStack(spacing: 0) {
                    globalContextStrip
                    HairlineDivider()
                    resetAlertBanner
                    AnalyticsView(store: store)
                }
                .background(SemanticColors.surfacePrimary)
                .navigationTitle(MainTab.analytics.navTitle)
                .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label(MainTab.analytics.title, systemImage: MainTab.analytics.systemImage)
            }
            .tag(MainTab.analytics)

            NavigationStack {
                VStack(spacing: 0) {
                    globalContextStrip
                    HairlineDivider()
                    resetAlertBanner
                    QuotasAndModelsView(store: store)
                }
                .background(SemanticColors.surfacePrimary)
                .navigationTitle(MainTab.quotas.navTitle)
                .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label(MainTab.quotas.title, systemImage: MainTab.quotas.systemImage)
            }
            .tag(MainTab.quotas)

            NavigationStack {
                SettingsView(store: store)
                    .background(SemanticColors.surfacePrimary)
                    .navigationTitle(MainTab.settings.navTitle)
                    .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label(MainTab.settings.title, systemImage: MainTab.settings.systemImage)
            }
            .tag(MainTab.settings)
        }
    }

    private var iPadLayout: some View {
        NavigationSplitView {
            List(MainTab.allCases, selection: Binding(
                get: { store.selectedTab },
                set: { if let v = $0 { store.selectedTab = v } }
            )) { tab in
                Label(tab.navTitle, systemImage: tab.systemImage)
                    .tag(tab)
            }
            .navigationTitle(L10n.tr("app.name", default: "TokenVane"))
        } detail: {
            VStack(spacing: 0) {
                if store.selectedTab != .settings {
                    globalContextStrip
                    HairlineDivider()
                    resetAlertBanner
                }
                Group {
                    switch store.selectedTab {
                    case .overview:
                        OverviewView(store: store)
                    case .analytics:
                        AnalyticsView(store: store)
                    case .quotas:
                        QuotasAndModelsView(store: store)
                    case .settings:
                        SettingsView(store: store)
                    }
                }
            }
            .background(SemanticColors.surfacePrimary)
            .navigationTitle(store.selectedTab.navTitle)
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    @ViewBuilder
    private var resetAlertBanner: some View {
        if let banner = store.latestResetBanner {
            HStack(spacing: 8) {
                Circle()
                    .fill(SemanticColors.statusLive)
                    .frame(width: 6, height: 6)
                Text(banner)
                    .font(.caption)
                    .lineLimit(2)
                Spacer()
                Button {
                    store.dismissResetBanner()
                } label: {
                    Image(systemName: "xmark")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(SemanticColors.surfaceSubtle)
            HairlineDivider()
        }
    }

    private var globalContextStrip: some View {
        HStack(spacing: 8) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    // Device filter menu
                    Menu {
                        Button {
                            Task { await store.selectDevice(nil) }
                        } label: {
                            if store.selectedDeviceId == nil {
                                Label(L10n.tr("filter.all_devices", default: "全部设备"), systemImage: "checkmark")
                            } else {
                                Text(L10n.tr("filter.all_devices", default: "全部设备"))
                            }
                        }
                        if let devices = store.bootstrap?.syncedDevices {
                            ForEach(devices) { dev in
                                Button {
                                    Task { await store.selectDevice(dev.deviceId) }
                                } label: {
                                    if store.selectedDeviceId == dev.deviceId {
                                        Label(dev.deviceName, systemImage: "checkmark")
                                    } else {
                                        Text(dev.deviceName)
                                    }
                                }
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "desktopcomputer")
                                .font(.caption2)
                            Text(selectedDeviceLabel)
                                .font(.caption.weight(.medium))
                                .lineLimit(1)
                            Image(systemName: "chevron.down")
                                .font(.caption2.weight(.semibold))
                        }
                        .foregroundStyle(.primary)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                    }
                    .accessibilityIdentifier("global.devicePicker")

                    // Time range strip
                    HStack(spacing: 2) {
                        ForEach(TimeRangePreset.allCases) { preset in
                            Button {
                                Task { await store.selectTimeRange(preset) }
                            } label: {
                                Text(preset.title)
                                    .font(.caption2.weight(store.timeRange == preset ? .semibold : .regular))
                                    .foregroundStyle(store.timeRange == preset ? Color.primary : Color.secondary)
                                    .lineLimit(1)
                                    .padding(.horizontal, 7)
                                    .padding(.vertical, 5)
                                    .background(
                                        store.timeRange == preset
                                            ? SemanticColors.surfaceSubtle
                                            : Color.clear,
                                        in: RoundedRectangle(cornerRadius: 4, style: .continuous)
                                    )
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("range.\(preset.rawValue)")
                        }
                    }
                }
            }

            // Pinned right-side currency toggle & sync pulse (always visible on small iPhones)
            HStack(spacing: 6) {
                Button {
                    Task { await store.toggleCurrency() }
                } label: {
                    Text(store.currency)
                        .font(.caption2.weight(.semibold))
                        .tabularMetrics()
                        .foregroundStyle(SemanticColors.accentIndigo)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .overlay(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("global.currencyToggle")

                Button {
                    Task { await store.triggerManualSync() }
                } label: {
                    HStack(spacing: 4) {
                        Circle()
                            .fill(syncIndicatorColor)
                            .frame(width: 7, height: 7)
                        Text(syncIndicatorText)
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .padding(.horizontal, 4)
                    .padding(.vertical, 4)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("global.syncPulse")
            }
            .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(SemanticColors.surfacePrimary)
    }

    private var selectedDeviceLabel: String {
        guard let selected = store.selectedDeviceId else {
            return L10n.tr("filter.all_devices", default: "全部设备")
        }
        return store.bootstrap?.syncedDevices.first(where: { $0.deviceId == selected })?.deviceName ?? selected
    }

    private var syncIndicatorColor: Color {
        switch store.connectionState {
        case .live: return SemanticColors.statusLive
        case .syncing: return SemanticColors.statusWarning
        case .error: return SemanticColors.statusDanger
        case .offline: return Color.secondary.opacity(0.45)
        }
    }

    private var syncIndicatorText: String {
        switch store.connectionState {
        case .live: return L10n.tr("sync.status.live", default: "在线")
        case .syncing: return L10n.tr("sync.status.syncing", default: "同步中")
        case .error: return L10n.tr("sync.status.error", default: "异常")
        case .offline:
            return TokenVaneFormatters.formatRelativeSync(atMs: store.bootstrap?.syncConfig.lastSyncAtMs)
        }
    }
}
