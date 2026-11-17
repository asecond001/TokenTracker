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
    }

    private var iPhoneLayout: some View {
        TabView(selection: $store.selectedTab) {
            NavigationStack {
                VStack(spacing: 0) {
                    globalContextStrip
                    HairlineDivider()
                    OverviewView(store: store)
                }
                .background(SemanticColors.surfacePrimary)
                .navigationTitle("概览")
                .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label("概览", systemImage: MainTab.overview.systemImage)
            }
            .tag(MainTab.overview)

            NavigationStack {
                VStack(spacing: 0) {
                    globalContextStrip
                    HairlineDivider()
                    AnalyticsView(store: store)
                }
                .background(SemanticColors.surfacePrimary)
                .navigationTitle("趋势与月报")
                .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label("趋势", systemImage: MainTab.analytics.systemImage)
            }
            .tag(MainTab.analytics)

            NavigationStack {
                VStack(spacing: 0) {
                    globalContextStrip
                    HairlineDivider()
                    QuotasAndModelsView(store: store)
                }
                .background(SemanticColors.surfacePrimary)
                .navigationTitle("订阅与模型")
                .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label("订阅", systemImage: MainTab.quotas.systemImage)
            }
            .tag(MainTab.quotas)

            NavigationStack {
                SettingsView(store: store)
                    .background(SemanticColors.surfacePrimary)
                    .navigationTitle("同步与设置")
                    .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label("设置", systemImage: MainTab.settings.systemImage)
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
                Label(tab.title, systemImage: tab.systemImage)
                    .tag(tab)
            }
            .navigationTitle("TokenVane")
        } detail: {
            VStack(spacing: 0) {
                if store.selectedTab != .settings {
                    globalContextStrip
                    HairlineDivider()
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
            .navigationTitle(store.selectedTab.title)
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private var globalContextStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 10) {
                // Device filter menu
                Menu {
                    Button {
                        Task { await store.selectDevice(nil) }
                    } label: {
                        if store.selectedDeviceId == nil {
                            Label("全部设备", systemImage: "checkmark")
                        } else {
                            Text("全部设备")
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
                            .font(.system(size: 9, weight: .semibold))
                    }
                    .foregroundStyle(.primary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
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
                                .padding(.horizontal, 7)
                                .padding(.vertical, 4)
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

                // USD / CNY capsule toggle
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

                // Sync pulse indicator
                Button {
                    Task { await store.triggerManualSync() }
                } label: {
                    HStack(spacing: 5) {
                        Circle()
                            .fill(syncIndicatorColor)
                            .frame(width: 7, height: 7)
                        Text(syncIndicatorText)
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 4)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("global.syncPulse")
            }
            .padding(.horizontal, 16)
        }
        .frame(height: 38)
        .background(SemanticColors.surfacePrimary)
    }

    private var selectedDeviceLabel: String {
        guard let selected = store.selectedDeviceId else { return "全部设备" }
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
        case .live: return "实时"
        case .syncing: return "同步中"
        case .error: return "异常"
        case .offline:
            return TokenVaneFormatters.formatRelativeSync(atMs: store.bootstrap?.syncConfig.lastSyncAtMs)
        }
    }
}
