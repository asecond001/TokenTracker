import Foundation
import TokenVaneCore
import UIKit
import WidgetKit

public actor CoreClientActor {
    private let engine: MobileCoreEngine
    private let snapshotURL: URL?

    public init(dbPath: String, localDeviceName: String = UIDevice.current.name, snapshotURL: URL? = nil) throws {
        self.engine = try MobileCoreEngine.open(dbPath: dbPath, localDeviceName: localDeviceName)
        self.snapshotURL = snapshotURL
    }

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
    public func ingestPullResponse(bodyJSON: String, e2eePassphrase: String?) async throws -> FfiSyncIngestOutcome {
        let taskID = await MainActor.run {
            UIApplication.shared.beginBackgroundTask(withName: "tokenvane.ingest", expirationHandler: nil)
        }
        defer {
            if taskID != .invalid {
                Task { @MainActor in
                    UIApplication.shared.endBackgroundTask(taskID)
                }
            }
        }
        let outcome = try engine.ingestPullResponseJson(
            pullBodyJson: bodyJSON,
            e2eePassphrase: e2eePassphrase,
            nowMs: Self.nowMs()
        )
        _ = try? refreshWidgetSnapshot()
        return outcome
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
        _ = try? refreshWidgetSnapshot()
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
        _ = try? refreshWidgetSnapshot()
        return state
    }

    @discardableResult
    public func applyExchangeRatePayload(source: String, bodyJSON: String) throws -> Double {
        let updated = try engine.applyExchangeRatePayload(source: source, bodyJson: bodyJSON, nowMs: Self.nowMs())
        _ = try? refreshWidgetSnapshot()
        return updated
    }

    @discardableResult
    public func refreshWidgetSnapshot() throws -> WidgetDashboardSnapshot {
        let raw = try engine.buildWidgetSnapshotJson(
            nowMs: Self.nowMs(),
            tzOffsetMinutes: Self.currentTzOffsetMinutes
        )
        let snapshot: WidgetDashboardSnapshot = try decodeJSON(raw)
        try AppGroupPaths.writeWidgetSnapshot(snapshot, to: snapshotURL)
        WidgetCenter.shared.reloadAllTimelines()
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
