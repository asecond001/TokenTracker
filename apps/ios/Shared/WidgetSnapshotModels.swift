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
    public let dayKey: String?
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
        case dayKey = "day_key"
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
        dayKey: String? = nil,
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
        self.dayKey = dayKey
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

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.schemaVersion = try container.decode(UInt32.self, forKey: .schemaVersion)
        self.generatedAtMs = try container.decode(Int64.self, forKey: .generatedAtMs)
        self.dayKey = try container.decodeIfPresent(String.self, forKey: .dayKey)
        self.currency = try container.decode(String.self, forKey: .currency)
        self.usdToCnyRate = try container.decode(String.self, forKey: .usdToCnyRate)
        self.todayTokens = try container.decode(Int64.self, forKey: .todayTokens)
        self.todayCostMicroUsd = try container.decode(Int64.self, forKey: .todayCostMicroUsd)
        self.todayRequests = try container.decode(Int64.self, forKey: .todayRequests)
        self.monthTokens = try container.decode(Int64.self, forKey: .monthTokens)
        self.monthCostMicroUsd = try container.decode(Int64.self, forKey: .monthCostMicroUsd)
        self.activeDeviceCount = try container.decode(Int64.self, forKey: .activeDeviceCount)
        self.latestOutputTps = try container.decodeIfPresent(Double.self, forKey: .latestOutputTps)
        self.lastSyncAtMs = try container.decodeIfPresent(Int64.self, forKey: .lastSyncAtMs)
        self.topProviders = try container.decode([WidgetProviderShare].self, forKey: .topProviders)
        self.primaryQuotas = try container.decode([WidgetQuotaWindowItem].self, forKey: .primaryQuotas)
    }

    /// Returns a display-safe snapshot that zeroes daily/monthly counters across midnight or month
    /// boundaries and resets quota windows whose reset timestamp has already elapsed.
    public func effectiveForDisplay(at now: Date, calendar: Calendar = .current) -> WidgetDashboardSnapshot {
        let snapDate = Date(timeIntervalSince1970: Double(generatedAtMs) / 1000.0)
        let currentDayKey = TokenVaneFormatters.formatDayKey(now, timeZone: calendar.timeZone)
        let resolvedSnapDayKey: String
        if let dayKey, !dayKey.isEmpty {
            resolvedSnapDayKey = dayKey
        } else {
            resolvedSnapDayKey = TokenVaneFormatters.formatDayKey(snapDate, timeZone: calendar.timeZone)
        }

        let sameDay = (resolvedSnapDayKey == currentDayKey)
        let sameMonth: Bool
        if resolvedSnapDayKey.count >= 7 && currentDayKey.count >= 7 {
            sameMonth = resolvedSnapDayKey.prefix(7) == currentDayKey.prefix(7)
        } else {
            sameMonth = calendar.isDate(snapDate, equalTo: now, toGranularity: .month)
        }

        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        let adjustedQuotas = primaryQuotas.map { item -> WidgetQuotaWindowItem in
            if let resetsAtMs = item.resetsAtMs, resetsAtMs <= nowMs {
                return WidgetQuotaWindowItem(
                    deviceId: item.deviceId,
                    deviceDisplayName: item.deviceDisplayName,
                    planId: item.planId,
                    providerId: item.providerId,
                    planName: item.planName,
                    windowId: item.windowId,
                    windowLabel: item.windowLabel,
                    usedPercent: 0.0,
                    resetsAtMs: nil,
                    updatedAtMs: item.updatedAtMs
                )
            }
            return item
        }

        return WidgetDashboardSnapshot(
            schemaVersion: schemaVersion,
            generatedAtMs: generatedAtMs,
            dayKey: currentDayKey,
            currency: currency,
            usdToCnyRate: usdToCnyRate,
            todayTokens: sameDay ? todayTokens : 0,
            todayCostMicroUsd: sameDay ? todayCostMicroUsd : 0,
            todayRequests: sameDay ? todayRequests : 0,
            monthTokens: sameMonth ? monthTokens : 0,
            monthCostMicroUsd: sameMonth ? monthCostMicroUsd : 0,
            activeDeviceCount: activeDeviceCount,
            latestOutputTps: sameDay ? latestOutputTps : nil,
            lastSyncAtMs: lastSyncAtMs,
            topProviders: sameDay ? topProviders : [],
            primaryQuotas: adjustedQuotas
        )
    }

    public static var placeholder: WidgetDashboardSnapshot {
        let now = Date()
        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        return WidgetDashboardSnapshot(
            generatedAtMs: nowMs,
            dayKey: TokenVaneFormatters.formatDayKey(now),
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
