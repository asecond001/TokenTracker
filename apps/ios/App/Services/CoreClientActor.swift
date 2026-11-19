import Foundation
@preconcurrency import TokenVaneCore
import UIKit
import WidgetKit

public struct SyncIngestOutcomeDTO: Equatable, Sendable {
    public let itemsReceived: UInt64
    public let envelopesApplied: UInt64
    public let hourlyBucketsApplied: UInt64
    public let subscriptionSnapshotsApplied: UInt64
    public let paygSnapshotsApplied: UInt64
    public let nextSinceMs: Int64
    public let serverSeqMs: Int64
    public let hasMore: Bool
    public let appliedDeviceIds: [String]

    public init(
        itemsReceived: UInt64,
        envelopesApplied: UInt64,
        hourlyBucketsApplied: UInt64,
        subscriptionSnapshotsApplied: UInt64,
        paygSnapshotsApplied: UInt64 = 0,
        nextSinceMs: Int64,
        serverSeqMs: Int64,
        hasMore: Bool,
        appliedDeviceIds: [String]
    ) {
        self.itemsReceived = itemsReceived
        self.envelopesApplied = envelopesApplied
        self.hourlyBucketsApplied = hourlyBucketsApplied
        self.subscriptionSnapshotsApplied = subscriptionSnapshotsApplied
        self.paygSnapshotsApplied = paygSnapshotsApplied
        self.nextSinceMs = nextSinceMs
        self.serverSeqMs = serverSeqMs
        self.hasMore = hasMore
        self.appliedDeviceIds = appliedDeviceIds
    }
}

public actor CoreClientActor {
    private let engine: MobileCoreEngine
    private let snapshotURL: URL?

    public init(dbPath: String, localDeviceName: String = "iPhone", snapshotURL: URL? = nil) throws {
        self.engine = try MobileCoreEngine.open(dbPath: dbPath, localDeviceName: localDeviceName)
        self.snapshotURL = snapshotURL
    }

    @MainActor
    public static func openDefault() throws -> CoreClientActor {
        let dbURL = try AppGroupPaths.databaseFileURL()
        return try CoreClientActor(dbPath: dbURL.path, localDeviceName: UIDevice.current.name, snapshotURL: nil)
    }

    private static var currentTzOffsetMinutes: Int32 {
        Int32(TimeZone.current.secondsFromGMT() / 60)
    }

    private static func nowMs() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1000)
    }

    public func getBootstrap() throws -> MobileBootstrapStateDTO {
        let raw = try engine.getBootstrapJson(nowMs: Self.nowMs(), tzOffsetMinutes: Self.currentTzOffsetMinutes)
        return try decodeJSON(raw)
    }

    @discardableResult
    public func ingestPullResponse(bodyJSON: String, e2eePassphrase: String?) async throws -> SyncIngestOutcomeDTO {
        let rawTaskID = await MainActor.run {
            UIApplication.shared.beginBackgroundTask(withName: "tokenvane.ingest", expirationHandler: nil).rawValue
        }
        defer {
            let invalidRaw = UIBackgroundTaskIdentifier.invalid.rawValue
            if rawTaskID != invalidRaw {
                Task { @MainActor in
                    UIApplication.shared.endBackgroundTask(UIBackgroundTaskIdentifier(rawValue: rawTaskID))
                }
            }
        }
        let outcome = try engine.ingestPullResponseJson(
            pullBodyJson: bodyJSON,
            e2eePassphrase: e2eePassphrase,
            nowMs: Self.nowMs()
        )
        _ = try? refreshWidgetSnapshot(reloadTimelines: false)
        return SyncIngestOutcomeDTO(
            itemsReceived: outcome.itemsReceived,
            envelopesApplied: outcome.envelopesApplied,
            hourlyBucketsApplied: outcome.hourlyBucketsApplied,
            subscriptionSnapshotsApplied: outcome.subscriptionSnapshotsApplied,
            paygSnapshotsApplied: outcome.paygSnapshotsApplied,
            nextSinceMs: outcome.nextSinceMs,
            serverSeqMs: outcome.serverSeqMs,
            hasMore: outcome.hasMore,
            appliedDeviceIds: outcome.appliedDeviceIds
        )
    }

    public func queryOverview(filter: AnalyticsFilterDTO) throws -> OverviewReportDTO {
        let filterJSON = try encodeJSON(filter)
        let raw = try engine.queryOverviewJson(filterJson: filterJSON, nowMs: Self.nowMs())
        return try decodeJSON(raw)
    }

    public func queryTrends(query: TrendsQueryDTO) throws -> TrendsReportDTO {
        let queryJSON = try encodeJSON(query)
        let raw = try engine.queryTrendsJson(queryJson: queryJSON)
        return try decodeJSON(raw)
    }

    public func queryMonthly(query: MonthlyQueryDTO) throws -> MonthlyReportDTO {
        let queryJSON = try encodeJSON(query)
        let raw = try engine.queryMonthlyJson(queryJson: queryJSON)
        return try decodeJSON(raw)
    }

    public func queryModelsPricing(filter: AnalyticsFilterDTO) throws -> ModelStatsReportDTO {
        let filterJSON = try encodeJSON(filter)
        let raw = try engine.queryModelsPricingJson(filterJson: filterJSON)
        return try decodeJSON(raw)
    }

    public func querySpeedLatency(filter: AnalyticsFilterDTO) throws -> SpeedLatencyReportDTO {
        let filterJSON = try encodeJSON(filter)
        let raw = try engine.querySpeedLatencyJson(filterJson: filterJSON)
        return try decodeJSON(raw)
    }

    public func queryRemoteSubscriptions() throws -> [RemoteSubscriptionSnapshotDTO] {
        let raw = try engine.queryRemoteSubscriptionsJson()
        return try decodeJSON(raw)
    }

    public func exportMonthlyReport(query: MonthlyQueryDTO, format: String) throws -> String {
        let queryJSON = try encodeJSON(query)
        return try engine.exportMonthlyReport(queryJson: queryJSON, format: format)
    }

    @discardableResult
    public func updatePreferences(_ patch: MobilePreferencesPatchDTO) throws -> AppPreferencesDTO {
        let patchJSON = try encodeJSON(patch)
        let raw = try engine.updateMobilePreferencesJson(patchJson: patchJSON, nowMs: Self.nowMs())
        let prefs: AppPreferencesDTO = try decodeJSON(raw)
        _ = try? refreshWidgetSnapshot(reloadTimelines: false)
        return prefs
    }

    @discardableResult
    public func updateSyncConfig(enabled: Bool, workerURL: String?, e2eeEnabled: Bool) throws -> SyncConfigStateDTO {
        let raw = try engine.updateSyncConfig(
            enabled: enabled,
            workerUrl: workerURL,
            e2eeEnabled: e2eeEnabled,
            nowMs: Self.nowMs()
        )
        return try decodeJSON(raw)
    }

    @discardableResult
    public func resetSyncWatermark() throws -> SyncConfigStateDTO {
        let raw = try engine.resetSyncWatermark(nowMs: Self.nowMs())
        let state: SyncConfigStateDTO = try decodeJSON(raw)
        _ = try? refreshWidgetSnapshot(reloadTimelines: false)
        return state
    }

    @discardableResult
    public func applyExchangeRatePayload(source: String, bodyJSON: String) throws -> Double {
        let updated = try engine.applyExchangeRatePayload(source: source, bodyJson: bodyJSON, nowMs: Self.nowMs())
        _ = try? refreshWidgetSnapshot(reloadTimelines: false)
        return updated
    }

    @discardableResult
    public func applyPricingCatalogPayload(
        source: String,
        bodyJSON: String,
        effectiveDate: String,
        nowMs: Int64? = nil
    ) throws -> UInt64 {
        try engine.applyPricingCatalogPayload(
            source: source,
            bodyJson: bodyJSON,
            effectiveDate: effectiveDate,
            nowMs: nowMs ?? Self.nowMs()
        )
    }

    @discardableResult
    public func applySubscriptionCatalogPayload(bodyJSON: String, nowMs: Int64? = nil) throws -> UInt64 {
        try engine.applySubscriptionCatalogPayload(
            bodyJson: bodyJSON,
            nowMs: nowMs ?? Self.nowMs()
        )
    }

    public func upsertModelPricing(_ entry: ModelPricingEntryDTO) throws {
        let json = try encodeJSON(entry)
        try engine.upsertModelPricingJson(entryJson: json)
    }

    public func deleteModelPricingOverride(modelPattern: String) throws {
        try engine.deleteModelPricingOverride(modelPattern: modelPattern)
    }

    public func queryPaygAccounts(nowMs: Int64? = nil) throws -> [PaygAccountStatusDTO] {
        let raw = try engine.queryPaygAccountsJson(
            nowMs: nowMs ?? Self.nowMs(),
            tzOffsetMinutes: Self.currentTzOffsetMinutes
        )
        return try decodeJSON(raw)
    }

    public func upsertPaygAccount(_ account: PaygAccountDTO) throws {
        let json = try encodeJSON(account)
        try engine.upsertPaygAccountJson(accountJson: json)
    }

    public func deletePaygAccount(accountId: String) throws {
        try engine.deletePaygAccount(accountId: accountId)
    }

    @discardableResult
    public func applyOfficialBalancePayload(
        providerId: String,
        endpoint: String,
        bodyJSON: String,
        nowMs: Int64? = nil
    ) throws -> OfficialBalanceApplyResultDTO {
        let raw = try engine.applyOfficialBalancePayloadJson(
            providerId: providerId,
            endpoint: endpoint,
            bodyJson: bodyJSON,
            nowMs: nowMs ?? Self.nowMs()
        )
        return try decodeJSON(raw)
    }

    public func listSubscriptionTemplates(nowMs: Int64? = nil) throws -> [SubscriptionPlanTemplateDTO] {
        let raw = try engine.listSubscriptionTemplatesJson(nowMs: nowMs ?? Self.nowMs())
        return try decodeJSON(raw)
    }

    public func queryLocalSubscriptions(nowMs: Int64? = nil) throws -> [SubscriptionPlanStatusDTO] {
        let raw = try engine.queryLocalSubscriptionsJson(
            nowMs: nowMs ?? Self.nowMs(),
            tzOffsetMinutes: Self.currentTzOffsetMinutes
        )
        return try decodeJSON(raw)
    }

    public func upsertSubscriptionPlan(_ plan: SubscriptionPlanDTO) throws {
        let json = try encodeJSON(plan)
        try engine.upsertSubscriptionPlanJson(planJson: json)
    }

    public func deleteSubscriptionPlan(planId: String) throws {
        try engine.deleteSubscriptionPlan(planId: planId)
    }

    public func buildWorkerUploadBundle(
        workerName: String? = nil,
        bearerToken: String? = nil,
        e2eeEnabled: Bool = false,
        e2eePassphrase: String? = nil,
        includeMigration: Bool = true
    ) throws -> CloudflareWorkerUploadBundleDTO {
        let raw = try engine.buildWorkerUploadBundleJson(
            workerName: workerName,
            bearerToken: bearerToken,
            e2eeEnabled: e2eeEnabled,
            e2eePassphrase: e2eePassphrase,
            includeMigration: includeMigration
        )
        return try decodeJSON(raw)
    }

    /// Writes the < 8 KB App Group snapshot. Intermediate writes (each pull page, preference
    /// patches) pass `reloadTimelines: false`; `AppStore.reloadAllData` asks WidgetKit to reload
    /// once per refresh cycle so the system widget reload budget is not exhausted.
    @discardableResult
    public func refreshWidgetSnapshot(reloadTimelines: Bool = true) throws -> WidgetDashboardSnapshot {
        let raw = try engine.buildWidgetSnapshotJson(
            nowMs: Self.nowMs(),
            tzOffsetMinutes: Self.currentTzOffsetMinutes
        )
        let snapshot: WidgetDashboardSnapshot = try decodeJSON(raw)
        try AppGroupPaths.writeWidgetSnapshot(snapshot, to: snapshotURL)
        if reloadTimelines {
            WidgetCenter.shared.reloadAllTimelines()
        }
        return snapshot
    }

    public func checkpointForSuspend() {
        try? engine.checkpointForSuspend()
    }

    private func encodeJSON<T: Encodable>(_ value: T) throws -> String {
        let data = try JSONEncoder().encode(value)
        guard let str = String(data: data, encoding: .utf8) else {
            throw NSError(domain: "TokenVaneCore", code: 1, userInfo: [NSLocalizedDescriptionKey: "Failed to encode JSON"])
        }
        return str
    }

    private func decodeJSON<T: Decodable>(_ raw: String) throws -> T {
        guard let data = raw.data(using: .utf8) else {
            throw NSError(domain: "TokenVaneCore", code: 2, userInfo: [NSLocalizedDescriptionKey: "Invalid UTF-8 JSON"])
        }
        return try JSONDecoder().decode(T.self, from: data)
    }
}
