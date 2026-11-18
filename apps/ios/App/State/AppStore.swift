import Foundation
import Observation
import SwiftUI

public enum TimeRangePreset: String, CaseIterable, Identifiable, Sendable {
    case today
    case sevenDays
    case thirtyDays
    case month
    case all

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .today: return L10n.tr("range.today", default: "今日")
        case .sevenDays: return L10n.tr("range.7d", default: "7日")
        case .thirtyDays: return L10n.tr("range.30d", default: "30日")
        case .month: return L10n.tr("range.month", default: "本月")
        case .all: return L10n.tr("range.all", default: "全部")
        }
    }

    public func makeFilter(providerId: String?, deviceId: String?, now: Date = Date()) -> AnalyticsFilterDTO {
        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        let cal = Calendar.current
        let tzOffset = Int32(TimeZone.current.secondsFromGMT(for: now) / 60)
        let startDate: Date
        switch self {
        case .today:
            startDate = cal.startOfDay(for: now)
        case .sevenDays:
            startDate = cal.date(byAdding: .day, value: -6, to: cal.startOfDay(for: now)) ?? now.addingTimeInterval(-7 * 86_400)
        case .thirtyDays:
            startDate = cal.date(byAdding: .day, value: -29, to: cal.startOfDay(for: now)) ?? now.addingTimeInterval(-30 * 86_400)
        case .month:
            let comps = cal.dateComponents([.year, .month], from: now)
            startDate = cal.date(from: comps) ?? cal.startOfDay(for: now)
        case .all:
            startDate = Date(timeIntervalSince1970: 0)
        }
        return AnalyticsFilterDTO(
            startMs: Int64(startDate.timeIntervalSince1970 * 1000),
            endMs: nowMs,
            providerId: providerId,
            deviceId: deviceId,
            tzOffsetMinutes: tzOffset,
            allTime: self == .all
        )
    }
}

public enum MainTab: String, CaseIterable, Identifiable, Hashable, Sendable {
    case overview
    case analytics
    case quotas
    case settings

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .overview: return L10n.tr("tab.overview", default: "概览")
        case .analytics: return L10n.tr("tab.analytics", default: "趋势")
        case .quotas: return L10n.tr("tab.quotas", default: "订阅")
        case .settings: return L10n.tr("tab.settings", default: "设置")
        }
    }

    public var navTitle: String {
        switch self {
        case .overview: return L10n.tr("nav.overview", default: "概览")
        case .analytics: return L10n.tr("nav.analytics", default: "趋势与月报")
        case .quotas: return L10n.tr("nav.quotas", default: "订阅与模型")
        case .settings: return L10n.tr("nav.settings", default: "同步与设置")
        }
    }

    public var systemImage: String {
        switch self {
        case .overview: return "gauge.with.dots.needle.33percent"
        case .analytics: return "chart.bar.xaxis"
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
    public var trendGranularity: TrendGranularityDTO = .auto
    public var trendDimension: StackDimensionDTO = .provider
    public var selectedYear: Int32
    public var selectedMonth: UInt32

    public var bootstrap: MobileBootstrapStateDTO?
    public var overview: OverviewReportDTO?
    public var trends: TrendsReportDTO?
    public var monthly: MonthlyReportDTO?
    public var modelsReport: ModelStatsReportDTO?
    public var speedReport: SpeedLatencyReportDTO?
    public var connectionState: SyncConnectionState = .offline
    public var pinnedWatchKeys: Set<String> = []
    public var lastErrorMessage: String?
    public var isLoading = false
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

    /// Monotonic tokens: newer loads drop stale results; targeted reloads avoid full 5-query sweeps.
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
        let comps = Calendar.current.dateComponents([.year, .month], from: now)
        self.selectedYear = Int32(comps.year ?? 2026)
        self.selectedMonth = UInt32(comps.month ?? 1)
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

    public var currentFilter: AnalyticsFilterDTO {
        timeRange.makeFilter(providerId: selectedProviderId, deviceId: selectedDeviceId)
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
            let trendsQuery = TrendsQueryDTO(
                filter: filter,
                granularity: trendGranularity,
                dimension: trendDimension
            )
            let monthlyQuery = MonthlyQueryDTO(
                year: selectedYear,
                month: selectedMonth,
                providerId: selectedProviderId,
                deviceId: selectedDeviceId
            )

            async let ov = core.queryOverview(filter: filter)
            async let tr = core.queryTrends(query: trendsQuery)
            async let mo = core.queryMonthly(query: monthlyQuery)
            async let md = core.queryModelsPricing(filter: filter)
            async let sp = core.querySpeedLatency(filter: filter)

            let results = try await (ov, tr, mo, md, sp)

            // A newer reload (filter change, sync update) superseded this one; drop stale results.
            guard generation == reloadGeneration else { return }

            self.bootstrap = boot
            self.overview = results.0
            if tGen == trendsGeneration {
                self.trends = results.1
            }
            if mGen == monthlyGeneration {
                self.monthly = results.2
            }
            self.modelsReport = results.3
            self.speedReport = results.4
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
        let query = TrendsQueryDTO(
            filter: currentFilter,
            granularity: trendGranularity,
            dimension: trendDimension
        )
        do {
            let report = try await core.queryTrends(query: query)
            guard gen == trendsGeneration else { return }
            self.trends = report
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
        let query = MonthlyQueryDTO(
            year: selectedYear,
            month: selectedMonth,
            providerId: selectedProviderId,
            deviceId: selectedDeviceId
        )
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

    public func selectTimeRange(_ range: TimeRangePreset) async {
        timeRange = range
        await reloadAllData()
    }

    public func selectTrendDimension(_ dimension: StackDimensionDTO) async {
        trendDimension = dimension
        await reloadTrendsOnly()
    }

    public func selectTrendGranularity(_ granularity: TrendGranularityDTO) async {
        trendGranularity = granularity
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
        await reloadMonthlyOnly()
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
        let query = MonthlyQueryDTO(
            year: selectedYear,
            month: selectedMonth,
            providerId: selectedProviderId,
            deviceId: selectedDeviceId
        )
        do {
            let content = try await core.exportMonthlyReport(query: query, format: format)
            let ext = format.lowercased() == "json" ? "json" : "csv"
            let fileName = String(format: "tokenvane-%04d-%02d.%@", selectedYear, selectedMonth, ext)
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(fileName)
            try content.write(to: url, atomically: true, encoding: .utf8)
            return url
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
                                "source": "local_estimation"
                            ],
                            [
                                "plan_id": "chatgpt_pro",
                                "provider_id": "codex",
                                "plan_name": "ChatGPT Pro",
                                "window_id": "7d",
                                "window_label": "7d",
                                "used_percent": 34.2,
                                "resets_at_ms": resetsSevenDays,
                                "source": "official_header"
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
