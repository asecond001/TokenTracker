import Foundation

public struct WidgetProviderShare: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let displayName: String
    public let tokens: Int64
    public let costMicroUsd: Int64
    public let sharePercent: Double

    public var id: String { providerId }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case displayName = "display_name"
        case tokens
        case costMicroUsd = "cost_micro_usd"
        case sharePercent = "share_percent"
    }

    public init(
        providerId: String,
        displayName: String,
        tokens: Int64,
        costMicroUsd: Int64,
        sharePercent: Double
    ) {
        self.providerId = providerId
        self.displayName = displayName
        self.tokens = tokens
        self.costMicroUsd = costMicroUsd
        self.sharePercent = sharePercent
    }
}

public struct WidgetQuotaWindowItem: Codable, Equatable, Identifiable, Sendable {
    public let deviceId: String
    public let deviceDisplayName: String
    public let planId: String
    public let providerId: String
    public let planName: String
    public let windowId: String
    public let windowLabel: String
    public let usedPercent: Double
    public let resetsAtMs: Int64?
    public let updatedAtMs: Int64

    public var id: String { "\(deviceId)::\(planId)::\(windowId)" }

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceDisplayName = "device_display_name"
        case planId = "plan_id"
        case providerId = "provider_id"
        case planName = "plan_name"
        case windowId = "window_id"
        case windowLabel = "window_label"
        case usedPercent = "used_percent"
        case resetsAtMs = "resets_at_ms"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        deviceId: String,
        deviceDisplayName: String,
        planId: String,
        providerId: String,
        planName: String,
        windowId: String,
        windowLabel: String,
        usedPercent: Double,
        resetsAtMs: Int64?,
        updatedAtMs: Int64
    ) {
        self.deviceId = deviceId
        self.deviceDisplayName = deviceDisplayName
        self.planId = planId
        self.providerId = providerId
        self.planName = planName
        self.windowId = windowId
        self.windowLabel = windowLabel
        self.usedPercent = usedPercent
        self.resetsAtMs = resetsAtMs
        self.updatedAtMs = updatedAtMs
    }
}

public struct WidgetDashboardSnapshot: Codable, Equatable, Sendable {
    public let schemaVersion: UInt32
    public let generatedAtMs: Int64
    public let currency: String
    public let usdToCnyRate: String
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let todayRequests: Int64
    public let monthTokens: Int64
    public let monthCostMicroUsd: Int64
    public let activeDeviceCount: Int64
    public let latestOutputTps: Double?
    public let lastSyncAtMs: Int64?
    public let topProviders: [WidgetProviderShare]
    public let primaryQuotas: [WidgetQuotaWindowItem]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case generatedAtMs = "generated_at_ms"
        case currency
        case usdToCnyRate = "usd_to_cny_rate"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case todayRequests = "today_requests"
        case monthTokens = "month_tokens"
        case monthCostMicroUsd = "month_cost_micro_usd"
        case activeDeviceCount = "active_device_count"
        case latestOutputTps = "latest_output_tps"
        case lastSyncAtMs = "last_sync_at_ms"
        case topProviders = "top_providers"
        case primaryQuotas = "primary_quotas"
    }

    public init(
        schemaVersion: UInt32 = 1,
        generatedAtMs: Int64,
        currency: String,
        usdToCnyRate: String,
        todayTokens: Int64,
        todayCostMicroUsd: Int64,
        todayRequests: Int64,
        monthTokens: Int64,
        monthCostMicroUsd: Int64,
        activeDeviceCount: Int64,
        latestOutputTps: Double?,
        lastSyncAtMs: Int64?,
        topProviders: [WidgetProviderShare],
        primaryQuotas: [WidgetQuotaWindowItem]
    ) {
        self.schemaVersion = schemaVersion
        self.generatedAtMs = generatedAtMs
        self.currency = currency
        self.usdToCnyRate = usdToCnyRate
        self.todayTokens = todayTokens
        self.todayCostMicroUsd = todayCostMicroUsd
        self.todayRequests = todayRequests
        self.monthTokens = monthTokens
        self.monthCostMicroUsd = monthCostMicroUsd
        self.activeDeviceCount = activeDeviceCount
        self.latestOutputTps = latestOutputTps
        self.lastSyncAtMs = lastSyncAtMs
        self.topProviders = topProviders
        self.primaryQuotas = primaryQuotas
    }

    public static var placeholder: WidgetDashboardSnapshot {
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        return WidgetDashboardSnapshot(
            generatedAtMs: nowMs,
            currency: "USD",
            usdToCnyRate: "7.25",
            todayTokens: 3_420_000,
            todayCostMicroUsd: 14_280_000,
            todayRequests: 148,
            monthTokens: 48_900_000,
            monthCostMicroUsd: 186_400_000,
            activeDeviceCount: 2,
            latestOutputTps: 68.4,
            lastSyncAtMs: nowMs - 45_000,
            topProviders: [
                WidgetProviderShare(
                    providerId: "claude_code",
                    displayName: "Claude Code",
                    tokens: 2_100_000,
                    costMicroUsd: 9_800_000,
                    sharePercent: 61.4
                ),
                WidgetProviderShare(
                    providerId: "codex",
                    displayName: "Codex",
                    tokens: 920_000,
                    costMicroUsd: 3_200_000,
                    sharePercent: 26.9
                ),
                WidgetProviderShare(
                    providerId: "antigravity",
                    displayName: "Antigravity",
                    tokens: 400_000,
                    costMicroUsd: 1_280_000,
                    sharePercent: 11.7
                )
            ],
            primaryQuotas: [
                WidgetQuotaWindowItem(
                    deviceId: "win-workstation",
                    deviceDisplayName: "Windows-Workstation",
                    planId: "claude_max_5x",
                    providerId: "claude_code",
                    planName: "Claude Max",
                    windowId: "5h",
                    windowLabel: "5h",
                    usedPercent: 78.2,
                    resetsAtMs: nowMs + 4_320_000,
                    updatedAtMs: nowMs
                ),
                WidgetQuotaWindowItem(
                    deviceId: "macbook-pro",
                    deviceDisplayName: "MacBook-Pro",
                    planId: "chatgpt_pro",
                    providerId: "codex",
                    planName: "ChatGPT Pro",
                    windowId: "7d",
                    windowLabel: "7d",
                    usedPercent: 41.0,
                    resetsAtMs: nowMs + 86_400_000,
                    updatedAtMs: nowMs
                )
            ]
        )
    }
}
