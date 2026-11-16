import Foundation

public struct AnalyticsFilterDTO: Codable, Equatable, Sendable {
    public let startMs: Int64
    public let endMs: Int64
    public let providerId: String?
    public let deviceId: String?
    public let tzOffsetMinutes: Int32
    public let allTime: Bool

    enum CodingKeys: String, CodingKey {
        case startMs = "start_ms"
        case endMs = "end_ms"
        case providerId = "provider_id"
        case deviceId = "device_id"
        case tzOffsetMinutes = "tz_offset_minutes"
        case allTime = "all_time"
    }

    public init(
        startMs: Int64,
        endMs: Int64,
        providerId: String? = nil,
        deviceId: String? = nil,
        tzOffsetMinutes: Int32 = Int32(TimeZone.current.secondsFromGMT() / 60),
        allTime: Bool = false
    ) {
        self.startMs = startMs
        self.endMs = endMs
        self.providerId = providerId
        self.deviceId = deviceId
        self.tzOffsetMinutes = tzOffsetMinutes
        self.allTime = allTime
    }
}

public enum TrendGranularityDTO: String, Codable, CaseIterable, Sendable {
    case auto
    case fifteenMinutes = "fifteen_minutes"
    case hourly
    case daily
    case weekly

    public var label: String {
        switch self {
        case .auto: return "Auto"
        case .fifteenMinutes: return "15m"
        case .hourly: return "小时"
        case .daily: return "天"
        case .weekly: return "周"
        }
    }
}

public enum StackDimensionDTO: String, Codable, CaseIterable, Sendable {
    case provider
    case model
    case tokenType = "token_type"

    public var label: String {
        switch self {
        case .provider: return "Provider"
        case .model: return "Model"
        case .tokenType: return "TokenType"
        }
    }
}

public struct TrendsQueryDTO: Codable, Equatable, Sendable {
    public let filter: AnalyticsFilterDTO
    public let granularity: TrendGranularityDTO
    public let dimension: StackDimensionDTO

    public init(
        filter: AnalyticsFilterDTO,
        granularity: TrendGranularityDTO,
        dimension: StackDimensionDTO
    ) {
        self.filter = filter
        self.granularity = granularity
        self.dimension = dimension
    }
}

public struct MonthlyQueryDTO: Codable, Equatable, Sendable {
    public let year: Int32
    public let month: UInt32
    public let providerId: String?
    public let deviceId: String?
    public let tzOffsetMinutes: Int32

    enum CodingKeys: String, CodingKey {
        case year
        case month
        case providerId = "provider_id"
        case deviceId = "device_id"
        case tzOffsetMinutes = "tz_offset_minutes"
    }

    public init(
        year: Int32,
        month: UInt32,
        providerId: String? = nil,
        deviceId: String? = nil,
        tzOffsetMinutes: Int32 = Int32(TimeZone.current.secondsFromGMT() / 60)
    ) {
        self.year = year
        self.month = month
        self.providerId = providerId
        self.deviceId = deviceId
        self.tzOffsetMinutes = tzOffsetMinutes
    }
}

public struct SyncedDevicePulseDTO: Codable, Equatable, Identifiable, Sendable {
    public let deviceId: String
    public let deviceName: String
    public let platform: String
    public let lastActivityMs: Int64?
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let totalTokens: Int64
    public let totalCostMicroUsd: Int64

    public var id: String { deviceId }

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceName = "device_name"
        case platform
        case lastActivityMs = "last_activity_ms"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case totalTokens = "total_tokens"
        case totalCostMicroUsd = "total_cost_micro_usd"
    }
}

public struct AppPreferencesDTO: Codable, Equatable, Sendable {
    public var currency: String
    public var usdToCnyRate: String
    public var theme: String
    public var minimizeToTray: Bool
    public var launchAtStartup: Bool
    public var otlpEnabled: Bool
    public var otlpPort: UInt16
    public var deviceDisplayName: String
    public var bgMode: String
    public var bgImagePath: String?
    public var bgPresetId: String
    public var bgOpacity: Double
    public var bgBlurPx: Int32
    public var autoSyncPricing: Bool
    public var exchangeRateAuto: Bool
    public var autoCheckUpdates: Bool

    enum CodingKeys: String, CodingKey {
        case currency
        case usdToCnyRate = "usd_to_cny_rate"
        case theme
        case minimizeToTray = "minimize_to_tray"
        case launchAtStartup = "launch_at_startup"
        case otlpEnabled = "otlp_enabled"
        case otlpPort = "otlp_port"
        case deviceDisplayName = "device_display_name"
        case bgMode = "bg_mode"
        case bgImagePath = "bg_image_path"
        case bgPresetId = "bg_preset_id"
        case bgOpacity = "bg_opacity"
        case bgBlurPx = "bg_blur_px"
        case autoSyncPricing = "auto_sync_pricing"
        case exchangeRateAuto = "exchange_rate_auto"
        case autoCheckUpdates = "auto_check_updates"
    }
}

public struct SyncConfigStateDTO: Codable, Equatable, Sendable {
    public let enabled: Bool
    public let workerUrl: String?
    public let e2eeEnabled: Bool
    public let e2eeKdfSaltHex: String?
    public let lastPushWatermarkMs: Int64
    public let lastPullWatermarkMs: Int64
    public let lastSyncAtMs: Int64?
    public let lastSyncError: String?
    public let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case enabled
        case workerUrl = "worker_url"
        case e2eeEnabled = "e2ee_enabled"
        case e2eeKdfSaltHex = "e2ee_kdf_salt_hex"
        case lastPushWatermarkMs = "last_push_watermark_ms"
        case lastPullWatermarkMs = "last_pull_watermark_ms"
        case lastSyncAtMs = "last_sync_at_ms"
        case lastSyncError = "last_sync_error"
        case updatedAtMs = "updated_at_ms"
    }
}

public struct RemoteSubscriptionSnapshotDTO: Codable, Equatable, Identifiable, Sendable {
    public let deviceId: String
    public let deviceDisplayName: String
    public let planId: String
    public let windowId: String
    public let providerId: String
    public let planName: String
    public let windowLabel: String
    public let usedPercent: Double
    public let resetsAtMs: Int64?
    public let source: String
    public let updatedAtMs: Int64

    public var id: String { "\(deviceId)::\(planId)::\(windowId)" }

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceDisplayName = "device_display_name"
        case planId = "plan_id"
        case windowId = "window_id"
        case providerId = "provider_id"
        case planName = "plan_name"
        case windowLabel = "window_label"
        case usedPercent = "used_percent"
        case resetsAtMs = "resets_at_ms"
        case source
        case updatedAtMs = "updated_at_ms"
    }
}

public struct MobileBootstrapStateDTO: Codable, Equatable, Sendable {
    public let schemaVersion: UInt32
    public let viewerDeviceId: String
    public let preferences: AppPreferencesDTO
    public let syncConfig: SyncConfigStateDTO
    public let syncedDevices: [SyncedDevicePulseDTO]
    public let remoteSubscriptions: [RemoteSubscriptionSnapshotDTO]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case viewerDeviceId = "viewer_device_id"
        case preferences
        case syncConfig = "sync_config"
        case syncedDevices = "synced_devices"
        case remoteSubscriptions = "remote_subscriptions"
    }
}

public struct MobilePreferencesPatchDTO: Codable, Equatable, Sendable {
    public let currency: String?
    public let usdToCnyRate: String?
    public let theme: String?
    public let exchangeRateAuto: Bool?

    enum CodingKeys: String, CodingKey {
        case currency
        case usdToCnyRate = "usd_to_cny_rate"
        case theme
        case exchangeRateAuto = "exchange_rate_auto"
    }

    public init(
        currency: String? = nil,
        usdToCnyRate: String? = nil,
        theme: String? = nil,
        exchangeRateAuto: Bool? = nil
    ) {
        self.currency = currency
        self.usdToCnyRate = usdToCnyRate
        self.theme = theme
        self.exchangeRateAuto = exchangeRateAuto
    }
}

public struct TimeBucketPointDTO: Codable, Equatable, Identifiable, Sendable {
    public let bucketStartMs: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let cacheWriteTokens: Int64
    public let reasoningTokens: Int64
    public let costMicroUsd: Int64
    public let requests: Int64

    public var id: Int64 { bucketStartMs }

    enum CodingKeys: String, CodingKey {
        case bucketStartMs = "bucket_start_ms"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheWriteTokens = "cache_write_tokens"
        case reasoningTokens = "reasoning_tokens"
        case costMicroUsd = "cost_micro_usd"
        case requests
    }
}

public struct UsageShareItemDTO: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let label: String
    public let tokens: Int64
    public let tokenSharePercent: Double
    public let costMicroUsd: Int64
    public let costSharePercent: Double

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case label
        case tokens
        case tokenSharePercent = "token_share_percent"
        case costMicroUsd = "cost_micro_usd"
        case costSharePercent = "cost_share_percent"
    }
}

public struct ProviderSummaryRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let enabled: Bool
    public let periodTokens: Int64
    public let periodCostMicroUsd: Int64
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let monthTokens: Int64
    public let monthCostMicroUsd: Int64
    public let cacheHitRatio: Double
    public let observableSteps: Int64
    public let activeWindowLabel: String?
    public let activeWindowUsedPercent: Double?
    public let lastSyncAtMs: Int64?
    public let lastError: String?

    public var id: String { providerId }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case enabled
        case periodTokens = "period_tokens"
        case periodCostMicroUsd = "period_cost_micro_usd"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case monthTokens = "month_tokens"
        case monthCostMicroUsd = "month_cost_micro_usd"
        case cacheHitRatio = "cache_hit_ratio"
        case observableSteps = "observable_steps"
        case activeWindowLabel = "active_window_label"
        case activeWindowUsedPercent = "active_window_used_percent"
        case lastSyncAtMs = "last_sync_at_ms"
        case lastError = "last_error"
    }
}

public struct TrendPointDTO: Codable, Equatable, Identifiable, Sendable {
    public let bucketStartMs: Int64
    public let tokens: Int64
    public let costMicroUsd: Int64
    public let requests: Int64

    public var id: Int64 { bucketStartMs }

    enum CodingKeys: String, CodingKey {
        case bucketStartMs = "bucket_start_ms"
        case tokens
        case costMicroUsd = "cost_micro_usd"
        case requests
    }
}

public struct TrendSeriesDTO: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let label: String
    public let totalTokens: Int64
    public let totalCostMicroUsd: Int64
    public let points: [TrendPointDTO]

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case label
        case totalTokens = "total_tokens"
        case totalCostMicroUsd = "total_cost_micro_usd"
        case points
    }
}

public struct TrendComparisonRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let label: String
    public let currentTokens: Int64
    public let previousTokens: Int64
    public let deltaPct: Double?
    public let currentCostMicroUsd: Int64
    public let previousCostMicroUsd: Int64
    public let requests: Int64

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case label
        case currentTokens = "current_tokens"
        case previousTokens = "previous_tokens"
        case deltaPct = "delta_pct"
        case currentCostMicroUsd = "current_cost_micro_usd"
        case previousCostMicroUsd = "previous_cost_micro_usd"
        case requests
    }
}

public struct OverviewReportDTO: Codable, Equatable, Sendable {
    public let totalTokens: Int64
    public let totalCostMicroUsd: Int64
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let cacheHitRatio: Double
    public let avgOutputTps: Double
    public let peakOutputTps: Double
    public let latestOutputTps: Double?
    public let totalRequests: Int64
    public let observableSteps: Int64
    public let tokensDeltaPct: Double?
    public let costDeltaPct: Double?
    public let cacheDeltaPct: Double?
    public let tpsDeltaPct: Double?
    public let seriesGranularity: String
    public let series: [TimeBucketPointDTO]
    public let providerSeries: [TrendSeriesDTO]
    public let providers: [ProviderSummaryRowDTO]
    public let providerShares: [UsageShareItemDTO]
    public let topModelShares: [UsageShareItemDTO]

    enum CodingKeys: String, CodingKey {
        case totalTokens = "total_tokens"
        case totalCostMicroUsd = "total_cost_micro_usd"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case cacheHitRatio = "cache_hit_ratio"
        case avgOutputTps = "avg_output_tps"
        case peakOutputTps = "peak_output_tps"
        case latestOutputTps = "latest_output_tps"
        case totalRequests = "total_requests"
        case observableSteps = "observable_steps"
        case tokensDeltaPct = "tokens_delta_pct"
        case costDeltaPct = "cost_delta_pct"
        case cacheDeltaPct = "cache_delta_pct"
        case tpsDeltaPct = "tps_delta_pct"
        case seriesGranularity = "series_granularity"
        case series
        case providerSeries = "provider_series"
        case providers
        case providerShares = "provider_shares"
        case topModelShares = "top_model_shares"
    }
}

public struct TrendsReportDTO: Codable, Equatable, Sendable {
    public let effectiveGranularity: TrendGranularityDTO
    public let buckets: [Int64]
    public let series: [TrendSeriesDTO]
    public let comparisons: [TrendComparisonRowDTO]

    enum CodingKeys: String, CodingKey {
        case effectiveGranularity = "effective_granularity"
        case buckets
        case series
        case comparisons
    }
}

public struct MonthlyDailyLedgerRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let date: String
    public let requests: Int64
    public let observableSteps: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let cacheWriteTokens: Int64
    public let reasoningTokens: Int64
    public let totalTokens: Int64
    public let costMicroUsd: Int64
    public let cacheSavedMicroUsd: Int64
    public let topModel: String?

    public var id: String { date }

    enum CodingKeys: String, CodingKey {
        case date
        case requests
        case observableSteps = "observable_steps"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheWriteTokens = "cache_write_tokens"
        case reasoningTokens = "reasoning_tokens"
        case totalTokens = "total_tokens"
        case costMicroUsd = "cost_micro_usd"
        case cacheSavedMicroUsd = "cache_saved_micro_usd"
        case topModel = "top_model"
    }
}

public struct HeatmapDayCellDTO: Codable, Equatable, Identifiable, Sendable {
    public let date: String
    public let totalTokens: Int64
    public let costMicroUsd: Int64
    public let requests: Int64
    public let topModel: String?

    public var id: String { date }

    enum CodingKeys: String, CodingKey {
        case date
        case totalTokens = "total_tokens"
        case costMicroUsd = "cost_micro_usd"
        case requests
        case topModel = "top_model"
    }
}

public struct MonthlyReportDTO: Codable, Equatable, Sendable {
    public let year: Int32
    public let month: UInt32
    public let apiEquivalentCostMicroUsd: Int64
    public let subscriptionMonthlyFeeMicroUsd: Int64
    public let roiMultiplier: Double?
    public let monthTotalTokens: Int64
    public let monthTotalRequests: Int64
    public let monthCacheSavedMicroUsd: Int64
    public let dailyLedger: [MonthlyDailyLedgerRowDTO]
    public let heatmap365d: [HeatmapDayCellDTO]

    enum CodingKeys: String, CodingKey {
        case year
        case month
        case apiEquivalentCostMicroUsd = "api_equivalent_cost_micro_usd"
        case subscriptionMonthlyFeeMicroUsd = "subscription_monthly_fee_micro_usd"
        case roiMultiplier = "roi_multiplier"
        case monthTotalTokens = "month_total_tokens"
        case monthTotalRequests = "month_total_requests"
        case monthCacheSavedMicroUsd = "month_cache_saved_micro_usd"
        case dailyLedger = "daily_ledger"
        case heatmap365d = "heatmap_365d"
    }
}

public struct ModelStatRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let modelId: String
    public let providerId: String
    public let requests: Int64
    public let observableSteps: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let cacheWriteTokens: Int64
    public let reasoningTokens: Int64
    public let totalTokens: Int64
    public let cacheSavedMicroUsd: Int64
    public let estimatedCostMicroUsd: Int64
    public let costShareRatio: Double
    public let avgOutputTps: Double

    public var id: String { "\(providerId)::\(modelId)" }

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id"
        case providerId = "provider_id"
        case requests
        case observableSteps = "observable_steps"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheWriteTokens = "cache_write_tokens"
        case reasoningTokens = "reasoning_tokens"
        case totalTokens = "total_tokens"
        case cacheSavedMicroUsd = "cache_saved_micro_usd"
        case estimatedCostMicroUsd = "estimated_cost_micro_usd"
        case costShareRatio = "cost_share_ratio"
        case avgOutputTps = "avg_output_tps"
    }

    public var cacheHitPercent: Double {
        let denom = inputTokens + cacheReadTokens + cacheWriteTokens
        guard denom > 0 else { return 0 }
        return (Double(cacheReadTokens) / Double(denom)) * 100.0
    }
}

public struct ModelStatsReportDTO: Codable, Equatable, Sendable {
    public let rows: [ModelStatRowDTO]
    public let totalCostMicroUsd: Int64
    public let totalCacheSavedMicroUsd: Int64

    enum CodingKeys: String, CodingKey {
        case rows
        case totalCostMicroUsd = "total_cost_micro_usd"
        case totalCacheSavedMicroUsd = "total_cache_saved_micro_usd"
    }
}

public struct ModelSpeedStatsDTO: Codable, Equatable, Identifiable, Sendable {
    public let modelId: String
    public let providerId: String
    public let sampleCount: Int64
    public let meanTps: Double
    public let p50Tps: Double
    public let p95Tps: Double
    public let maxTps: Double
    public let minTps: Double
    public let latestTps: Double
    public let avgDurationMs: Double
    public let avgTtftMs: Double?

    public var id: String { "\(providerId)::\(modelId)" }

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id"
        case providerId = "provider_id"
        case sampleCount = "sample_count"
        case meanTps = "mean_tps"
        case p50Tps = "p50_tps"
        case p95Tps = "p95_tps"
        case maxTps = "max_tps"
        case minTps = "min_tps"
        case latestTps = "latest_tps"
        case avgDurationMs = "avg_duration_ms"
        case avgTtftMs = "avg_ttft_ms"
    }
}

public struct ProviderSpeedStatsDTO: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let sampleCount: Int64
    public let meanTps: Double
    public let p50Tps: Double
    public let p95Tps: Double
    public let maxTps: Double
    public let minTps: Double
    public let avgDurationMs: Double
    public let avgTtftMs: Double?

    public var id: String { providerId }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case sampleCount = "sample_count"
        case meanTps = "mean_tps"
        case p50Tps = "p50_tps"
        case p95Tps = "p95_tps"
        case maxTps = "max_tps"
        case minTps = "min_tps"
        case avgDurationMs = "avg_duration_ms"
        case avgTtftMs = "avg_ttft_ms"
    }
}

public struct SpeedLatencyReportDTO: Codable, Equatable, Sendable {
    public let providers: [ProviderSpeedStatsDTO]
    public let models: [ModelSpeedStatsDTO]
    public let fromHourlyRollups: Bool

    enum CodingKeys: String, CodingKey {
        case providers
        case models
        case fromHourlyRollups = "from_hourly_rollups"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.providers = try container.decode([ProviderSpeedStatsDTO].self, forKey: .providers)
        self.models = try container.decode([ModelSpeedStatsDTO].self, forKey: .models)
        self.fromHourlyRollups = try container.decodeIfPresent(Bool.self, forKey: .fromHourlyRollups) ?? false
    }

    public init(
        providers: [ProviderSpeedStatsDTO],
        models: [ModelSpeedStatsDTO],
        fromHourlyRollups: Bool
    ) {
        self.providers = providers
        self.models = models
        self.fromHourlyRollups = fromHourlyRollups
    }
}

public struct DesktopPairingPayload: Codable, Equatable, Sendable {
    public let v: Int
    public let url: String
    public let token: String
    public let e2ee: Bool
    public let key: String?

    public init(v: Int, url: String, token: String, e2ee: Bool, key: String? = nil) {
        self.v = v
        self.url = url
        self.token = token
        self.e2ee = e2ee
        self.key = key
    }

    public enum PairingError: LocalizedError, Equatable {
        case invalidJSON
        case unsupportedVersion(Int)
        case invalidURL
        case emptyToken

        public var errorDescription: String? {
            switch self {
            case .invalidJSON: return "配对二维码或链接格式无效"
            case .unsupportedVersion(let v): return "不支持的配对协议版本 v\(v)"
            case .invalidURL: return "Worker 地址必须为有效的 https:// 或本地 http:// URL"
            case .emptyToken: return "Sync Token 不能为空"
            }
        }
    }

    public static func parse(_ raw: String) throws -> DesktopPairingPayload {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let decoded: DesktopPairingPayload
        if trimmed.lowercased().hasPrefix("tokentracker://") {
            guard let components = URLComponents(string: trimmed),
                  (components.host?.lowercased() == "pair" || components.path.lowercased().contains("pair")),
                  let items = components.queryItems else {
                throw PairingError.invalidJSON
            }
            let map = Dictionary(items.compactMap { item -> (String, String)? in
                guard let value = item.value else { return nil }
                return (item.name.lowercased(), value)
            }, uniquingKeysWith: { _, latest in latest })

            let version = Int(map["v"] ?? "1") ?? 0
            let urlVal = map["url"] ?? map["u"] ?? ""
            let tokenVal = map["token"] ?? map["t"] ?? ""
            let e2eeRaw = (map["e2ee"] ?? map["e"] ?? "0").lowercased()
            let e2eeVal = e2eeRaw == "1" || e2eeRaw == "true" || e2eeRaw == "yes"
            let keyVal = (map["key"] ?? map["k"])?.trimmingCharacters(in: .whitespacesAndNewlines)
            decoded = DesktopPairingPayload(
                v: version,
                url: urlVal,
                token: tokenVal,
                e2ee: e2eeVal,
                key: keyVal?.isEmpty == true ? nil : keyVal
            )
        } else {
            guard let data = trimmed.data(using: .utf8),
                  let jsonPayload = try? JSONDecoder().decode(DesktopPairingPayload.self, from: data) else {
                throw PairingError.invalidJSON
            }
            decoded = jsonPayload
        }
        guard decoded.v == 1 else {
            throw PairingError.unsupportedVersion(decoded.v)
        }
        let cleanedURL = decoded.url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = URL(string: cleanedURL),
              let scheme = parsed.scheme?.lowercased(),
              (scheme == "https" || (scheme == "http" && (parsed.host == "127.0.0.1" || parsed.host == "localhost"))),
              parsed.host != nil else {
            throw PairingError.invalidURL
        }
        let cleanedToken = decoded.token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanedToken.isEmpty else {
            throw PairingError.emptyToken
        }
        let cleanedKey = decoded.key?.trimmingCharacters(in: .whitespacesAndNewlines)
        return DesktopPairingPayload(
            v: decoded.v,
            url: cleanedURL.hasSuffix("/") ? String(cleanedURL.dropLast()) : cleanedURL,
            token: cleanedToken,
            e2ee: decoded.e2ee,
            key: cleanedKey?.isEmpty == true ? nil : cleanedKey
        )
    }
}
