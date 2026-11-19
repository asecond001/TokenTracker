import Foundation
import Observation
import SwiftUI

public struct CustomTimeRangeState: Equatable, Sendable {
    public var startDate: Date
    public var endDate: Date
    public var isPrecise: Bool
    public var startMs: Int64?
    public var endMs: Int64?

    public init(
        startDate: Date,
        endDate: Date,
        isPrecise: Bool = false,
        startMs: Int64? = nil,
        endMs: Int64? = nil
    ) {
        self.startDate = startDate
        self.endDate = endDate
        self.isPrecise = isPrecise
        self.startMs = startMs
        self.endMs = endMs
    }
}

public enum TimeRangePreset: String, CaseIterable, Identifiable, Sendable {
    case today
    case sevenDays
    case thirtyDays
    case month
    case all
    case custom

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .today: return L10n.tr("range.today", default: "今日")
        case .sevenDays: return L10n.tr("range.7d", default: "7日")
        case .thirtyDays: return L10n.tr("range.30d", default: "30日")
        case .month: return L10n.tr("range.month", default: "本月")
        case .all: return L10n.tr("range.all", default: "全部")
        case .custom: return L10n.tr("range.custom", default: "自定义")
        }
    }

    public func makeFilter(
        providerId: String?,
        deviceId: String?,
        customRange: CustomTimeRangeState? = nil,
        now: Date = Date()
    ) -> AnalyticsFilterDTO {
        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        let cal = Calendar.current
        let tzOffset = Int32(TimeZone.current.secondsFromGMT(for: now) / 60)
        let todayStart = cal.startOfDay(for: now)
        let startMs: Int64
        let endMs: Int64

        switch self {
        case .today:
            startMs = Int64(todayStart.timeIntervalSince1970 * 1000)
            endMs = nowMs
        case .sevenDays:
            let startDate = cal.date(byAdding: .day, value: -6, to: todayStart) ?? now.addingTimeInterval(-6 * 86_400)
            startMs = Int64(startDate.timeIntervalSince1970 * 1000)
            endMs = Self.endOfDayMs(for: now, calendar: cal)
        case .thirtyDays:
            let startDate = cal.date(byAdding: .day, value: -29, to: todayStart) ?? now.addingTimeInterval(-29 * 86_400)
            startMs = Int64(startDate.timeIntervalSince1970 * 1000)
            endMs = Self.endOfDayMs(for: now, calendar: cal)
        case .month:
            let comps = cal.dateComponents([.year, .month], from: now)
            let startDate = cal.date(from: comps) ?? todayStart
            startMs = Int64(startDate.timeIntervalSince1970 * 1000)
            endMs = Self.endOfDayMs(for: now, calendar: cal)
        case .all:
            startMs = 0
            endMs = nowMs
        case .custom:
            if let customRange, customRange.isPrecise, let preciseStart = customRange.startMs {
                let rawEnd = customRange.endMs ?? nowMs
                startMs = min(preciseStart, rawEnd)
                endMs = max(preciseStart, rawEnd)
            } else if let customRange {
                let d1 = cal.startOfDay(for: customRange.startDate)
                let d2 = cal.startOfDay(for: customRange.endDate)
                let firstDay = min(d1, d2)
                let lastDay = max(d1, d2)
                startMs = Int64(firstDay.timeIntervalSince1970 * 1000)
                endMs = Self.endOfDayMs(for: lastDay, calendar: cal)
            } else {
                let startDate = cal.date(byAdding: .day, value: -6, to: todayStart) ?? now.addingTimeInterval(-6 * 86_400)
                startMs = Int64(startDate.timeIntervalSince1970 * 1000)
                endMs = Self.endOfDayMs(for: now, calendar: cal)
            }
        }

        return AnalyticsFilterDTO(
            startMs: startMs,
            endMs: endMs,
            providerId: providerId,
            deviceId: deviceId,
            tzOffsetMinutes: tzOffset,
            allTime: self == .all
        )
    }

    private static func endOfDayMs(for date: Date, calendar: Calendar) -> Int64 {
        let dayStart = calendar.startOfDay(for: date)
        let nextDay = calendar.date(byAdding: .day, value: 1, to: dayStart) ?? dayStart.addingTimeInterval(86_400)
        return Int64(nextDay.timeIntervalSince1970 * 1000) - 1
    }
}

public enum TrendMetricMode: String, CaseIterable, Identifiable, Sendable {
    case tokens
    case cost
    case cacheHit = "cache_hit"
    case requests

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .tokens: return L10n.tr("trends.metric.tokens", default: "Tokens")
        case .cost: return L10n.tr("trends.metric.cost", default: "费用")
        case .cacheHit: return L10n.tr("trends.metric.cache_hit", default: "命中率")
        case .requests: return L10n.tr("trends.metric.requests", default: "请求")
        }
    }
}

public enum TrendChartStyleMode: String, CaseIterable, Identifiable, Sendable {
    case line
    case bar

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .line: return L10n.tr("trends.style.line", default: "折线")
        case .bar: return L10n.tr("trends.style.bar", default: "柱状")
        }
    }
}

public enum MainTab: String, CaseIterable, Identifiable, Hashable, Sendable {
    case overview
    case analytics
    case models
    case quotas
    case settings

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .overview: return L10n.tr("tab.overview", default: "概览")
        case .analytics: return L10n.tr("tab.analytics", default: "趋势")
        case .models: return L10n.tr("tab.models", default: "模型")
        case .quotas: return L10n.tr("tab.quotas", default: "订阅")
        case .settings: return L10n.tr("tab.settings", default: "设置")
        }
    }

    public var navTitle: String {
        switch self {
        case .overview: return L10n.tr("nav.overview", default: "概览")
        case .analytics: return L10n.tr("nav.analytics", default: "趋势与统计")
        case .models: return L10n.tr("nav.models", default: "模型对比")
        case .quotas: return L10n.tr("nav.quotas", default: "订阅与配额")
        case .settings: return L10n.tr("nav.settings", default: "同步与设置")
        }
    }

    public var systemImage: String {
        switch self {
        case .overview: return "gauge.with.dots.needle.33percent"
        case .analytics: return "chart.bar.xaxis"
        case .models: return "square.stack.3d.up"
        case .quotas: return "cpu"
        case .settings: return "arrow.triangle.2.circlepath.circle"
        }
    }
}

public struct LiveActivityFeedback: Equatable, Sendable {
    public let message: String
    public let canOpenSettings: Bool

    public init(message: String, canOpenSettings: Bool) {
        self.message = message
        self.canOpenSettings = canOpenSettings
    }
}

@Observable
@MainActor
public final class AppStore {
    private static let manualRateDraftKey = "tokenvane.manual_usd_to_cny_rate_draft"

    public var selectedTab: MainTab = .overview
    public var selectedDeviceId: String? = nil
    public var selectedProviderId: String? = nil
    public var timeRange: TimeRangePreset = .sevenDays
    public var customStartDate: Date
    public var customEndDate: Date
    public var customPrecise = false
    public var customStartMs: Int64? = nil
    public var customEndMs: Int64? = nil

    public var trendGranularity: TrendGranularityDTO = .auto
    public var trendDimension: StackDimensionDTO = .provider
    public var trendMetric: TrendMetricMode = .tokens
    public var trendChartStyle: TrendChartStyleMode = .line
    public var comparePreviousPeriod = true
    public var focusedSeriesKey: String? = nil
    public var selectedYear: Int32
    public var selectedMonth: UInt32

    public var bootstrap: MobileBootstrapStateDTO?
    public var overview: OverviewReportDTO?
    public var trends: TrendsReportDTO?
    public var previousPeriodTrends: TrendsReportDTO?
    public var monthly: MonthlyReportDTO?
    public var modelsReport: ModelStatsReportDTO?
    public var speedReport: SpeedLatencyReportDTO?
    public var localSubscriptions: [SubscriptionPlanStatusDTO] = []
    public var subscriptionTemplates: [SubscriptionPlanTemplateDTO] = []
    public var paygAccounts: [PaygAccountStatusDTO] = []
    public var connectionState: SyncConnectionState = .offline
    public var pinnedWatchKeys: Set<String> = []
    public var lastErrorMessage: String?
    public var isLoading = false
    public var isSyncingCatalogs = false
    public var isProvisioning = false
    public var provisionStep: CloudSyncProvisionStep? = nil
    public var latestPairingURI: String? = nil
    public var refreshingPaygAccountIds: Set<String> = []
    /// Set when the local SQLite core fails to open; views surface it instead of a blank screen.
    public private(set) var coreInitError: String?
    /// Pairing payload parsed from a deep link / QR / clipboard, awaiting user confirmation.
    public var pendingPairing: DesktopPairingPayload?
    public var pairingStatusMessage: String?
    /// Bumped after every successful sync-config save so editors can re-hydrate their fields.
    public private(set) var syncConfigRevision = 0
    /// Feedback surfaced when Live Activities cannot be started (e.g., disabled in Settings).
    public var liveActivityFeedback: LiveActivityFeedback?
    /// In-app banner shown when a subscription window resets or receives bonus quota.
    public var latestResetBanner: String?
    /// Preserved manual USD -> CNY rate so enabling auto FX does not erase user input.
    public var manualRateDraft: String

    public var resetNotificationsEnabled: Bool {
        get { resetNotifier.notificationsEnabled }
        set { resetNotifier.notificationsEnabled = newValue }
    }

    /// Monotonic tokens: newer loads drop stale results; targeted reloads avoid full queries.
    private var reloadGeneration = 0
    private var trendsGeneration = 0
    private var monthlyGeneration = 0

    private let core: CoreClientActor?
    private let secrets: SecretStoring
    private var syncCoordinator: SyncCoordinator?
    private let resetNotifier: QuotaResetNotifier
    private let userDefaults: UserDefaults
    private let isDemoMode: Bool
    private let stageDemoPairing: Bool

    public init(
        core: CoreClientActor? = nil,
        secrets: SecretStoring? = nil,
        syncCoordinator: SyncCoordinator? = nil,
        resetNotifier: QuotaResetNotifier? = nil,
        userDefaults: UserDefaults = .standard,
        demoMode: Bool = ProcessInfo.processInfo.arguments.contains("--uitest-seed-demo"),
        stageDemoPairing: Bool = ProcessInfo.processInfo.arguments.contains("--uitest-stage-pairing")
    ) {
        let now = Date()
        let cal = Calendar.current
        let todayStart = cal.startOfDay(for: now)
        let comps = cal.dateComponents([.year, .month], from: now)
        self.selectedYear = Int32(comps.year ?? 2026)
        self.selectedMonth = UInt32(comps.month ?? 1)
        self.customStartDate = cal.date(byAdding: .day, value: -6, to: todayStart) ?? todayStart
        self.customEndDate = todayStart
        self.userDefaults = userDefaults
        self.isDemoMode = demoMode
        self.stageDemoPairing = stageDemoPairing
        self.manualRateDraft = userDefaults.string(forKey: Self.manualRateDraftKey) ?? "7.25"
        self.resetNotifier = resetNotifier ?? QuotaResetNotifier.shared

        if let secrets {
            self.secrets = secrets
        } else if demoMode || stageDemoPairing {
            self.secrets = InMemorySecretStore()
        } else {
            self.secrets = KeychainStore.shared
        }

        var resolvedCore: CoreClientActor?
        var initError: String?
        if let provided = core {
            resolvedCore = provided
        } else {
            do {
                if demoMode || stageDemoPairing {
                    let demoURL = FileManager.default.temporaryDirectory
                        .appendingPathComponent("tokenvane-uitest-\(UUID().uuidString).sqlite")
                    resolvedCore = try CoreClientActor(dbPath: demoURL.path, localDeviceName: "iPhone-UITest")
                } else {
                    resolvedCore = try CoreClientActor.openDefault()
                }
            } catch {
                initError = error.localizedDescription
            }
        }
        self.core = resolvedCore
        self.coreInitError = initError

        if let providedSync = syncCoordinator {
            self.syncCoordinator = providedSync
        } else if let core = self.core {
            self.syncCoordinator = SyncCoordinator(core: core, secrets: self.secrets)
        }

        if let coordinator = self.syncCoordinator {
            coordinator.onStateChanged = { [weak self] state in
                self?.connectionState = state
            }
            coordinator.onDataUpdated = { [weak self] in
                await self?.reloadAllData()
            }
            coordinator.onBackgroundSyncCompleted = { [weak self] in
                await self?.refreshWidgetsAndActivitiesFromLocal()
            }
        }
    }

    public var currency: String {
        bootstrap?.preferences.currency ?? "USD"
    }

    public var usdToCnyRate: String {
        bootstrap?.preferences.usdToCnyRate ?? "7.25"
    }

    public var preferredColorScheme: ColorScheme? {
        switch bootstrap?.preferences.theme {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    public var syncConfigConfigured: Bool {
        guard let cfg = bootstrap?.syncConfig, cfg.enabled, let url = cfg.workerUrl else { return false }
        return !url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public var customRangeState: CustomTimeRangeState {
        CustomTimeRangeState(
            startDate: customStartDate,
            endDate: customEndDate,
            isPrecise: customPrecise,
            startMs: customStartMs,
            endMs: customEndMs
        )
    }

    public var currentFilter: AnalyticsFilterDTO {
        timeRange.makeFilter(
            providerId: selectedProviderId,
            deviceId: selectedDeviceId,
            customRange: customRangeState
        )
    }

    public var overviewTodayFilter: AnalyticsFilterDTO {
        TimeRangePreset.today.makeFilter(
            providerId: selectedProviderId,
            deviceId: selectedDeviceId
        )
    }

    public var resolvedDateSpanLabel: String {
        let now = Date()
        let filter = currentFilter
        if timeRange == .all {
            return L10n.tr("range.span.all", default: "全量历史")
        }
        if timeRange == .custom && customPrecise, customStartMs != nil {
            let sLabel = TokenVaneFormatters.formatBucketTimestamp(filter.startMs, isDateOnly: false)
            let eLabel = customEndMs == nil
                ? L10n.tr("range.span.now", default: "现在")
                : TokenVaneFormatters.formatBucketTimestamp(filter.endMs, isDateOnly: false)
            return "\(sLabel) ~ \(eLabel)"
        }
        let sStr = TokenVaneFormatters.formatDayKey(Date(timeIntervalSince1970: Double(filter.startMs) / 1000.0))
        let eStr = TokenVaneFormatters.formatDayKey(Date(timeIntervalSince1970: Double(min(filter.endMs, Int64(now.timeIntervalSince1970 * 1000))) / 1000.0))
        return sStr == eStr ? sStr : "\(sStr) ~ \(eStr)"
    }

    public var availableProviders: [String] {
        var seen = Set<String>()
        var result: [String] = []
        func appendIfNew(_ pid: String) {
            let trimmed = pid.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, seen.insert(trimmed).inserted else { return }
            result.append(trimmed)
        }
        for row in overview?.providers ?? [] { appendIfNew(row.providerId) }
        for row in modelsReport?.rows ?? [] { appendIfNew(row.providerId) }
        for sub in bootstrap?.remoteSubscriptions ?? [] { appendIfNew(sub.providerId) }
        for payg in bootstrap?.remotePaygAccounts ?? [] { appendIfNew(payg.providerId) }
        for sub in localSubscriptions { appendIfNew(sub.plan.providerId) }
        for payg in paygAccounts { appendIfNew(payg.account.providerId) }
        for builtIn in ["claude_code", "codex", "antigravity", "kimi_code", "deepseek_harness", "qcode", "minimax_code", "workbuddy"] {
            appendIfNew(builtIn)
        }
        return result
    }

    public func registerBackgroundTasks() {
        syncCoordinator?.registerBackgroundTasks()
    }

    public func bootstrapAndSeedIfNeeded() async {
        if isDemoMode {
            await seedDemoDataIntoEngine()
        }
        await reloadAllData()
        pinnedWatchKeys = LiveActivityManager.shared.activeWatchKeys()
        if stageDemoPairing && pendingPairing == nil {
            _ = stagePairing(
                raw: "tokenvane://pair?v=1&url=https%3A%2F%2Fuitest-sync.tokenvane.local&token=uitest-pairing-token&e2ee=0"
            )
        }
    }

    public func handleScenePhase(_ phase: ScenePhase) {
        switch phase {
        case .active:
            if !isDemoMode && !stageDemoPairing {
                syncCoordinator?.handleScenePhaseChanged(toForeground: true)
            }
            Task { await reloadAllData() }
        case .background:
            if !isDemoMode && !stageDemoPairing {
                syncCoordinator?.handleScenePhaseChanged(toForeground: false)
            } else {
                let coreRef = core
                Task { await coreRef?.checkpointForSuspend() }
            }
        case .inactive:
            break
        @unknown default:
            break
        }
    }

    public func reloadAllData() async {
        guard let core else { return }
        reloadGeneration &+= 1
        trendsGeneration &+= 1
        monthlyGeneration &+= 1
        let generation = reloadGeneration
        let tGen = trendsGeneration
        let mGen = monthlyGeneration
        isLoading = true
        defer {
            if generation == reloadGeneration {
                isLoading = false
            }
        }
        do {
            let boot = try await core.getBootstrap()

            if userDefaults.string(forKey: Self.manualRateDraftKey) == nil && !boot.preferences.exchangeRateAuto {
                manualRateDraft = boot.preferences.usdToCnyRate
                userDefaults.set(manualRateDraft, forKey: Self.manualRateDraftKey)
            }

            let filter = currentFilter
            let ovFilter = overviewTodayFilter
            let trendsQuery = TrendsQueryDTO(
                filter: filter,
                granularity: trendGranularity,
                dimension: trendDimension
            )
            let monthlyQuery = makeMonthlyQuery(from: filter)

            async let ov = core.queryOverview(filter: ovFilter)
            async let tr = core.queryTrends(query: trendsQuery)
            async let mo = core.queryMonthly(query: monthlyQuery)
            async let md = core.queryModelsPricing(filter: filter)
            async let sp = core.querySpeedLatency(filter: filter)
            async let localSubs = core.queryLocalSubscriptions()
            async let templates = core.listSubscriptionTemplates()
            async let payg = core.queryPaygAccounts()

            let results = try await (ov, tr, mo, md, sp, localSubs, templates, payg)

            var prevTrends: TrendsReportDTO? = nil
            if comparePreviousPeriod && !filter.allTime {
                let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
                let prevFilter = makePreviousPeriodFilter(from: filter, nowMs: nowMs)
                let prevQuery = TrendsQueryDTO(
                    filter: prevFilter,
                    granularity: results.1.effectiveGranularity,
                    dimension: trendDimension
                )
                prevTrends = try? await core.queryTrends(query: prevQuery)
            }

            // A newer reload (filter change, sync update) superseded this one; drop stale results.
            guard generation == reloadGeneration else { return }

            self.bootstrap = boot
            self.overview = results.0
            if tGen == trendsGeneration {
                self.trends = results.1
                self.previousPeriodTrends = prevTrends
            }
            if mGen == monthlyGeneration {
                self.monthly = results.2
            }
            self.modelsReport = results.3
            self.speedReport = results.4
            self.localSubscriptions = results.5
            self.subscriptionTemplates = results.6
            self.paygAccounts = results.7
            self.latestPairingURI = buildCurrentPairingURI(from: boot)
            self.lastErrorMessage = nil

            if !isDemoMode && !stageDemoPairing {
                let snapshot = try? await core.refreshWidgetSnapshot()
                await LiveActivityManager.shared.refreshActiveActivities(
                    subscriptions: boot.remoteSubscriptions,
                    snapshot: snapshot
                )
                pinnedWatchKeys = LiveActivityManager.shared.activeWatchKeys()
                await processResetNotifications(for: boot.remoteSubscriptions)
            }
        } catch {
            if generation == reloadGeneration {
                self.lastErrorMessage = error.localizedDescription
            }
        }
    }

    public func reloadTrendsOnly() async {
        guard let core else { return }
        trendsGeneration &+= 1
        let gen = trendsGeneration
        let filter = currentFilter
        let query = TrendsQueryDTO(
            filter: filter,
            granularity: trendGranularity,
            dimension: trendDimension
        )
        do {
            let report = try await core.queryTrends(query: query)
            var prevReport: TrendsReportDTO? = nil
            if comparePreviousPeriod && !filter.allTime {
                let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
                let prevFilter = makePreviousPeriodFilter(from: filter, nowMs: nowMs)
                let prevQuery = TrendsQueryDTO(
                    filter: prevFilter,
                    granularity: report.effectiveGranularity,
                    dimension: trendDimension
                )
                prevReport = try? await core.queryTrends(query: prevQuery)
            }
            guard gen == trendsGeneration else { return }
            self.trends = report
            self.previousPeriodTrends = prevReport
            self.lastErrorMessage = nil
        } catch {
            if gen == trendsGeneration {
                self.lastErrorMessage = error.localizedDescription
            }
        }
    }

    public func reloadMonthlyOnly() async {
        guard let core else { return }
        monthlyGeneration &+= 1
        let gen = monthlyGeneration
        let query = makeMonthlyQuery(from: currentFilter)
        do {
            let report = try await core.queryMonthly(query: query)
            guard gen == monthlyGeneration else { return }
            self.monthly = report
            self.lastErrorMessage = nil
        } catch {
            if gen == monthlyGeneration {
                self.lastErrorMessage = error.localizedDescription
            }
        }
    }

    private func makePreviousPeriodFilter(from filter: AnalyticsFilterDTO, nowMs: Int64) -> AnalyticsFilterDTO {
        let effectiveEnd = max(filter.startMs, min(filter.endMs, nowMs))
        let span = effectiveEnd - filter.startMs + 1
        return AnalyticsFilterDTO(
            startMs: filter.startMs - span,
            endMs: filter.startMs - 1,
            providerId: filter.providerId,
            deviceId: filter.deviceId,
            tzOffsetMinutes: filter.tzOffsetMinutes,
            allTime: false
        )
    }

    private func makeMonthlyQuery(from filter: AnalyticsFilterDTO, now: Date = Date()) -> MonthlyQueryDTO {
        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        let anchorMs = filter.allTime ? nowMs : min(filter.endMs, nowMs)
        let anchor = Date(timeIntervalSince1970: Double(anchorMs) / 1000.0)
        let comps = Calendar.current.dateComponents([.year, .month], from: anchor)
        return MonthlyQueryDTO(
            year: Int32(comps.year ?? Int(selectedYear)),
            month: UInt32(comps.month ?? Int(selectedMonth)),
            providerId: filter.providerId,
            deviceId: filter.deviceId,
            tzOffsetMinutes: filter.tzOffsetMinutes,
            startMs: filter.startMs,
            endMs: filter.endMs
        )
    }

    private func buildCurrentPairingURI(from boot: MobileBootstrapStateDTO) -> String? {
        guard boot.syncConfig.enabled,
              let url = boot.syncConfig.workerUrl,
              !url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }
        let token = loadSecretMasked(account: .syncBearerToken)
        guard !token.isEmpty else { return nil }
        let pass = boot.syncConfig.e2eeEnabled ? loadSecretMasked(account: .e2eePassphrase) : nil
        return DesktopPairingPayload.makePairingURI(
            url: url,
            token: token,
            e2ee: boot.syncConfig.e2eeEnabled,
            key: pass
        )
    }

    @discardableResult
    public func performBackgroundSyncRefresh() async -> Bool {
        guard !isDemoMode, let core else { return false }
        let ok = await syncCoordinator?.pullIncremental() ?? false
        if !ok {
            await refreshWidgetsAndActivitiesFromLocal()
        }
        await core.checkpointForSuspend()
        return ok
    }

    private func refreshWidgetsAndActivitiesFromLocal() async {
        guard let core, !isDemoMode else { return }
        if let boot = try? await core.getBootstrap() {
            let snapshot = try? await core.refreshWidgetSnapshot(reloadTimelines: true)
            await LiveActivityManager.shared.refreshActiveActivities(
                subscriptions: boot.remoteSubscriptions,
                snapshot: snapshot
            )
            await processResetNotifications(for: boot.remoteSubscriptions)
        }
    }

    private func processResetNotifications(for subscriptions: [RemoteSubscriptionSnapshotDTO]) async {
        let events = await resetNotifier.processSubscriptions(subscriptions)
        if let first = events.first {
            latestResetBanner = L10n.trFormat(
                "notification.quota_reset.banner",
                default: "%@ · %@ 已重置 (%lld%% → %lld%%)",
                first.planName,
                first.windowLabel,
                Int64(first.previousPercent.rounded()),
                Int64(first.currentPercent.rounded())
            )
        }
    }

    public func dismissResetBanner() {
        latestResetBanner = nil
    }

    public func dismissLiveActivityFeedback() {
        liveActivityFeedback = nil
    }

    public func toggleCurrency() async {
        let next = currency.uppercased() == "USD" ? "CNY" : "USD"
        await applyPreferencesPatch(MobilePreferencesPatchDTO(currency: next))
    }

    public func applyPreferencesPatch(_ patch: MobilePreferencesPatchDTO) async {
        guard let core else { return }
        do {
            _ = try await core.updatePreferences(patch)
            await reloadAllData()
        } catch {
            lastErrorMessage = error.localizedDescription
        }
    }

    @discardableResult
    public func applyManualExchangeRate(_ rawRate: String) async -> Bool {
        let trimmed = rawRate.trimmingCharacters(in: .whitespacesAndNewlines)
        guard TokenVaneFormatters.validateExchangeRate(trimmed) != nil else {
            return false
        }
        manualRateDraft = trimmed
        userDefaults.set(trimmed, forKey: Self.manualRateDraftKey)
        await applyPreferencesPatch(MobilePreferencesPatchDTO(usdToCnyRate: trimmed))
        return true
    }

    public func setAutoExchangeRate(_ enabled: Bool) async {
        if enabled {
            await applyPreferencesPatch(MobilePreferencesPatchDTO(exchangeRateAuto: true))
            _ = await syncCoordinator?.syncExchangeRateIfAuto()
        } else {
            let validDraft = TokenVaneFormatters.validateExchangeRate(manualRateDraft) != nil
                ? manualRateDraft
                : nil
            await applyPreferencesPatch(
                MobilePreferencesPatchDTO(
                    usdToCnyRate: validDraft,
                    exchangeRateAuto: false
                )
            )
        }
    }

    public func selectDevice(_ deviceId: String?) async {
        selectedDeviceId = deviceId
        await reloadAllData()
    }

    public func selectProvider(_ providerId: String?) async {
        selectedProviderId = providerId
        await reloadAllData()
    }

    public func selectTimeRange(_ range: TimeRangePreset) async {
        timeRange = range
        if range != .custom {
            customPrecise = false
            customStartMs = nil
            customEndMs = nil
        }
        await reloadAllData()
    }

    public func applyCustomDateRange(start: Date, end: Date) async {
        let cal = Calendar.current
        let d1 = cal.startOfDay(for: start)
        let d2 = cal.startOfDay(for: end)
        customStartDate = min(d1, d2)
        customEndDate = max(d1, d2)
        customPrecise = false
        customStartMs = nil
        customEndMs = nil
        timeRange = .custom
        await reloadAllData()
    }

    public func applyPreciseTimeRange(startMs: Int64, endMs: Int64?) async {
        let effectiveEndMs = endMs ?? Int64(Date().timeIntervalSince1970 * 1000)
        customStartDate = Date(timeIntervalSince1970: Double(min(startMs, effectiveEndMs)) / 1000.0)
        customEndDate = Date(timeIntervalSince1970: Double(max(startMs, effectiveEndMs)) / 1000.0)
        customPrecise = true
        customStartMs = min(startMs, endMs ?? startMs)
        customEndMs = endMs
        timeRange = .custom
        await reloadAllData()
    }

    public func selectTrendDimension(_ dimension: StackDimensionDTO) async {
        trendDimension = dimension
        focusedSeriesKey = nil
        await reloadTrendsOnly()
    }

    public func selectTrendGranularity(_ granularity: TrendGranularityDTO) async {
        trendGranularity = granularity
        await reloadTrendsOnly()
    }

    public func setComparePreviousPeriod(_ enabled: Bool) async {
        comparePreviousPeriod = enabled
        await reloadTrendsOnly()
    }

    public func shiftMonth(by delta: Int) async {
        var year = Int(selectedYear)
        var month = Int(selectedMonth) + delta
        while month < 1 {
            month += 12
            year -= 1
        }
        while month > 12 {
            month -= 12
            year += 1
        }
        selectedYear = Int32(year)
        selectedMonth = UInt32(month)

        var comps = DateComponents()
        comps.year = year
        comps.month = month
        comps.day = 1
        let cal = Calendar.current
        if let firstDay = cal.date(from: comps),
           let nextMonth = cal.date(byAdding: .month, value: 1, to: firstDay),
           let lastDay = cal.date(byAdding: .day, value: -1, to: nextMonth) {
            customStartDate = firstDay
            customEndDate = lastDay
            customPrecise = false
            customStartMs = nil
            customEndMs = nil
            timeRange = .custom
            await reloadAllData()
        } else {
            await reloadMonthlyOnly()
        }
    }

    public func triggerManualSync() async {
        if isDemoMode || stageDemoPairing {
            await reloadAllData()
            return
        }
        guard let syncCoordinator else { return }
        _ = await syncCoordinator.pullIncremental()
    }

    public func saveSyncSettings(
        enabled: Bool,
        workerURL: String,
        bearerToken: String,
        e2eeEnabled: Bool,
        e2eePassphrase: String
    ) async -> Bool {
        guard let core else { return false }
        do {
            if !bearerToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                try secrets.saveSecret(bearerToken, account: .syncBearerToken)
            }
            if !e2eePassphrase.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                try secrets.saveSecret(e2eePassphrase, account: .e2eePassphrase)
            } else if !e2eeEnabled {
                try secrets.deleteSecret(account: .e2eePassphrase)
            }
            _ = try await core.updateSyncConfig(
                enabled: enabled,
                workerURL: workerURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : workerURL,
                e2eeEnabled: e2eeEnabled
            )
            // Publish the new config before bumping the revision so editors re-hydrate from it,
            // and invalidate any in-flight reload that still carries the previous config.
            self.bootstrap = try await core.getBootstrap()
            reloadGeneration &+= 1
            syncConfigRevision &+= 1
            await reloadAllData()
            if enabled && !isDemoMode && !stageDemoPairing {
                await triggerManualSync()
                syncCoordinator?.startForegroundPolling()
            } else if !enabled {
                syncCoordinator?.stopForegroundPolling()
            }
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    /// Parses a pairing link / QR / clipboard payload and stages it for explicit confirmation.
    /// Nothing is written until `confirmPairing(_:)` runs, so an untrusted link cannot silently
    /// redirect sync to another Worker.
    @discardableResult
    public func stagePairing(raw: String) -> Bool {
        do {
            pendingPairing = try DesktopPairingPayload.parse(raw)
            return true
        } catch {
            pairingStatusMessage = error.localizedDescription
            return false
        }
    }

    public func cancelPendingPairing() {
        pendingPairing = nil
    }

    public func confirmPairing(_ payload: DesktopPairingPayload) async {
        pendingPairing = nil
        let ok = await saveSyncSettings(
            enabled: true,
            workerURL: payload.url,
            bearerToken: payload.token,
            e2eeEnabled: payload.e2ee,
            // Empty keeps any passphrase already stored in Keychain.
            e2eePassphrase: payload.key ?? ""
        )
        guard ok else {
            pairingStatusMessage = lastErrorMessage ?? L10n.tr("pairing.status.save_failed", default: "保存失败")
            return
        }
        let needsPassphrase = payload.e2ee && loadSecretMasked(account: .e2eePassphrase).isEmpty
        pairingStatusMessage = needsPassphrase
            ? L10n.tr("pairing.status.needs_passphrase", default: "已导入，请填写 E2EE 口令并保存")
            : L10n.tr("pairing.status.completed", default: "配对已完成")
    }

    public func loadSecretMasked(account: KeychainSecretAccount) -> String {
        (try? secrets.loadSecret(account: account)) ?? ""
    }

    public func testWorkerHealth(url: String, token: String) async -> Result<String, Error> {
        guard let syncCoordinator else {
            return .failure(NSError(domain: "AppStore", code: 2, userInfo: [NSLocalizedDescriptionKey: "SyncCoordinator unavailable"]))
        }
        return await syncCoordinator.probeHealth(workerURL: url, bearerToken: token)
    }

    public func syncExchangeRateNow() async -> Double? {
        await syncCoordinator?.syncExchangeRateNow()
    }

    public func resetSyncCache() async {
        guard let core else { return }
        do {
            resetNotifier.clearBaselines()
            _ = try await core.resetSyncWatermark()
            await reloadAllData()
        } catch {
            lastErrorMessage = error.localizedDescription
        }
    }

    public func toggleLiveActivityWatch(for sub: RemoteSubscriptionSnapshotDTO) async {
        liveActivityFeedback = nil
        let todayCost = TokenVaneFormatters.formatCurrency(
            overview?.todayCostMicroUsd ?? 0,
            currency: currency,
            rateString: usdToCnyRate
        )
        do {
            _ = try await LiveActivityManager.shared.toggleWatch(
                subscription: sub,
                todayCostFormatted: todayCost,
                latestTps: overview?.latestOutputTps
            )
            pinnedWatchKeys = LiveActivityManager.shared.activeWatchKeys()
        } catch let err as LiveActivityError {
            liveActivityFeedback = LiveActivityFeedback(
                message: err.localizedDescription,
                canOpenSettings: err == .activitiesDisabled
            )
        } catch {
            liveActivityFeedback = LiveActivityFeedback(
                message: error.localizedDescription,
                canOpenSettings: false
            )
            lastErrorMessage = error.localizedDescription
        }
    }

    public func generateMonthlyExportFile(format: String) async -> URL? {
        guard let core else { return nil }
        let query = makeMonthlyQuery(from: currentFilter)
        do {
            let content = try await core.exportMonthlyReport(query: query, format: format)
            let ext = format.lowercased() == "json" ? "json" : "csv"
            let fileName = String(format: "tokenvane-%04d-%02d.%@", query.year, query.month, ext)
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(fileName)
            try content.write(to: url, atomically: true, encoding: .utf8)
            return url
        } catch {
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    public func setBackdrop(mode: String, presetId: String, opacity: Double) async {
        await applyPreferencesPatch(
            MobilePreferencesPatchDTO(
                bgMode: mode,
                bgPresetId: presetId,
                bgOpacity: opacity
            )
        )
    }

    public func updateDeviceDisplayName(_ name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        await applyPreferencesPatch(MobilePreferencesPatchDTO(deviceDisplayName: trimmed))
    }

    @discardableResult
    public func syncAllCatalogsNow() async -> CatalogRefreshSummary? {
        guard let syncCoordinator else { return nil }
        isSyncingCatalogs = true
        defer { isSyncingCatalogs = false }
        let summary = await syncCoordinator.refreshPublicCatalogs(force: true)
        await reloadAllData()
        return summary
    }

    @discardableResult
    public func upsertModelPricing(_ entry: ModelPricingEntryDTO) async -> Bool {
        guard let core else { return false }
        do {
            try await core.upsertModelPricing(entry)
            await reloadAllData()
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func deleteModelPricingOverride(modelPattern: String) async -> Bool {
        guard let core else { return false }
        do {
            try await core.deleteModelPricingOverride(modelPattern: modelPattern)
            await reloadAllData()
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func upsertPaygAccount(_ account: PaygAccountDTO, apiKey: String? = nil) async -> Bool {
        guard let core else { return false }
        do {
            if let apiKey {
                let trimmedKey = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmedKey.isEmpty {
                    try secrets.deleteSecret(account: .providerApiKey(account.accountId))
                } else {
                    try secrets.saveSecret(trimmedKey, account: .providerApiKey(account.accountId))
                }
            }
            try await core.upsertPaygAccount(account)
            await reloadAllData()
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func deletePaygAccount(accountId: String) async -> Bool {
        guard let core else { return false }
        do {
            try? secrets.deleteSecret(account: .providerApiKey(accountId))
            try await core.deletePaygAccount(accountId: accountId)
            await reloadAllData()
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func refreshPaygBalance(_ account: PaygAccountDTO, bypassCooldown: Bool = false) async -> OfficialBalanceApplyResultDTO? {
        guard let syncCoordinator else { return nil }
        refreshingPaygAccountIds.insert(account.accountId)
        defer { refreshingPaygAccountIds.remove(account.accountId) }
        do {
            let result = try await syncCoordinator.refreshOfficialBalance(
                account: account,
                bypassCooldown: bypassCooldown
            )
            lastErrorMessage = nil
            await reloadAllData()
            return result
        } catch {
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    @discardableResult
    public func upsertSubscriptionPlan(_ plan: SubscriptionPlanDTO) async -> Bool {
        guard let core else { return false }
        do {
            try await core.upsertSubscriptionPlan(plan)
            await reloadAllData()
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func deleteSubscriptionPlan(planId: String) async -> Bool {
        guard let core else { return false }
        do {
            try await core.deleteSubscriptionPlan(planId: planId)
            await reloadAllData()
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func provisionWorker(
        workerName: String?,
        cfApiToken: String?,
        cfAccountId: String?,
        bearerToken: String?,
        e2eeEnabled: Bool,
        e2eePassphrase: String?
    ) async -> CloudSyncProvisionOutcome? {
        guard let syncCoordinator else { return nil }
        isProvisioning = true
        provisionStep = .preparing
        lastErrorMessage = nil
        defer {
            isProvisioning = false
        }
        do {
            let outcome = try await syncCoordinator.provisionCloudflareWorker(
                workerName: workerName,
                cfApiToken: cfApiToken,
                cfAccountId: cfAccountId,
                bearerToken: bearerToken,
                e2eeEnabled: e2eeEnabled,
                e2eePassphrase: e2eePassphrase,
                onProgress: { [weak self] step in
                    self?.provisionStep = step
                }
            )
            latestPairingURI = outcome.pairingURI
            syncConfigRevision &+= 1
            await reloadAllData()
            if !isDemoMode && !stageDemoPairing {
                syncCoordinator.startForegroundPolling()
            }
            return outcome
        } catch {
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    private func seedDemoDataIntoEngine() async {
        guard let core else { return }
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let hourMs: Int64 = 3_600_000
        let currentHour = (nowMs / hourMs) * hourMs
        let resetsFiveHours = nowMs + 3 * hourMs + 18 * 60_000
        let resetsSevenDays = nowMs + 4 * 86_400_000

        let pullPayload: [String: Any] = [
            "ok": true,
            "server_seq_ms": 999,
            "next_since_ms": 1000,
            "has_more": false,
            "items": [
                [
                    "mode": "plaintext",
                    "seq_ms": 500,
                    "device_id": "demo-win-ws",
                    "envelope": [
                        "schema_version": 1,
                        "device_id": "demo-win-ws",
                        "device_display_name": "Windows-Workstation",
                        "generated_at_ms": nowMs,
                        "since_watermark_ms": 0,
                        "hourly_deltas": [
                            [
                                "provider_id": "claude_code",
                                "model_id": "claude-sonnet-4-5",
                                "bucket_start_ms": currentHour - hourMs,
                                "request_count": 28,
                                "observable_step_count": 0,
                                "input_tokens": 820_000,
                                "output_tokens": 145_000,
                                "cache_read_tokens": 1_240_000,
                                "cache_write_tokens": 80_000,
                                "reasoning_tokens": 0,
                                "estimated_cost_micro_usd": 6_480_000,
                                "cache_saved_micro_usd": 1_850_000,
                                "total_duration_ms": 48_000,
                                "tps_sample_count": 28,
                                "tps_sum": 1960.0,
                                "max_output_tps": 84.2,
                                "updated_at_ms": nowMs - 120_000
                            ],
                            [
                                "provider_id": "codex",
                                "model_id": "gpt-5.3-codex",
                                "bucket_start_ms": currentHour,
                                "request_count": 19,
                                "observable_step_count": 0,
                                "input_tokens": 410_000,
                                "output_tokens": 92_000,
                                "cache_read_tokens": 600_000,
                                "cache_write_tokens": 0,
                                "reasoning_tokens": 35_000,
                                "estimated_cost_micro_usd": 3_120_000,
                                "cache_saved_micro_usd": 740_000,
                                "total_duration_ms": 29_000,
                                "tps_sample_count": 19,
                                "tps_sum": 1273.0,
                                "max_output_tps": 76.5,
                                "updated_at_ms": nowMs - 20_000
                            ]
                        ],
                        "subscription_snapshots": [
                            [
                                "plan_id": "claude_max_5x",
                                "provider_id": "claude_code",
                                "plan_name": "Claude Max 5x",
                                "window_id": "5h",
                                "window_label": "5h",
                                "used_percent": 78.4,
                                "resets_at_ms": resetsFiveHours,
                                "source": "local_estimation",
                                "monthly_fee_micro_usd": 200_000_000
                            ],
                            [
                                "plan_id": "chatgpt_pro",
                                "provider_id": "codex",
                                "plan_name": "ChatGPT Pro",
                                "window_id": "7d",
                                "window_label": "7d",
                                "used_percent": 34.2,
                                "resets_at_ms": resetsSevenDays,
                                "source": "official_header",
                                "monthly_fee_micro_usd": 200_000_000
                            ]
                        ],
                        "payg_snapshots": [
                            [
                                "account_id": "demo-ds-payg",
                                "provider_id": "deepseek_harness",
                                "account_name": "DeepSeek 官方账户",
                                "enabled": true,
                                "currency": "CNY",
                                "total_balance_micro": 125_400_000,
                                "granted_balance_micro": 20_000_000,
                                "topped_up_balance_micro": 105_400_000,
                                "remaining_manual_balance_micro_usd": NSNull(),
                                "monthly_budget_micro_usd": 50_000_000,
                                "today_cost_micro_usd": 1_850_000,
                                "month_cost_micro_usd": 18_420_000,
                                "fetched_at_ms": nowMs - 60_000,
                                "updated_at_ms": nowMs - 60_000
                            ]
                        ]
                    ]
                ],
                [
                    "mode": "plaintext",
                    "seq_ms": 999,
                    "device_id": "demo-mbp-m4",
                    "envelope": [
                        "schema_version": 1,
                        "device_id": "demo-mbp-m4",
                        "device_display_name": "MacBook-Pro",
                        "generated_at_ms": nowMs,
                        "since_watermark_ms": 0,
                        "hourly_deltas": [
                            [
                                "provider_id": "antigravity",
                                "model_id": "gemini-2.5-pro",
                                "bucket_start_ms": currentHour,
                                "request_count": 14,
                                "observable_step_count": 0,
                                "input_tokens": 510_000,
                                "output_tokens": 110_000,
                                "cache_read_tokens": 320_000,
                                "cache_write_tokens": 0,
                                "reasoning_tokens": 45_000,
                                "estimated_cost_micro_usd": 2_250_000,
                                "cache_saved_micro_usd": 310_000,
                                "total_duration_ms": 21_000,
                                "tps_sample_count": 14,
                                "tps_sum": 1148.0,
                                "max_output_tps": 96.4,
                                "updated_at_ms": nowMs - 10_000
                            ]
                        ],
                        "subscription_snapshots": []
                    ]
                ]
            ]
        ]

        if let data = try? JSONSerialization.data(withJSONObject: pullPayload),
           let body = String(data: data, encoding: .utf8) {
            _ = try? await core.ingestPullResponse(bodyJSON: body, e2eePassphrase: nil)
            _ = try? await core.updateSyncConfig(
                enabled: true,
                workerURL: "https://sync-demo.tokenvane.local",
                e2eeEnabled: false
            )
            connectionState = .live
        }
    }
}
