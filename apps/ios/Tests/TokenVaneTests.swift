import CommonCrypto
import CryptoKit
import Foundation
import XCTest
@testable import TokenVane

private final class MockProtocolRegistry: @unchecked Sendable {
    static let shared = MockProtocolRegistry()
    private let lock = NSLock()
    private var handlers: [String: @Sendable (URLRequest) throws -> (HTTPURLResponse, Data)] = [:]

    func setHandler(
        id: String,
        handler: (@Sendable (URLRequest) throws -> (HTTPURLResponse, Data))?
    ) {
        lock.lock()
        defer { lock.unlock() }
        if let handler {
            handlers[id] = handler
        } else {
            handlers.removeValue(forKey: id)
        }
    }

    func handler(for id: String) -> (@Sendable (URLRequest) throws -> (HTTPURLResponse, Data))? {
        lock.lock()
        defer { lock.unlock() }
        return handlers[id]
    }
}

private final class MockURLProtocol: URLProtocol {
    override class func canInit(with request: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        let id = request.value(forHTTPHeaderField: "X-Mock-Id") ?? ""
        guard let handler = MockProtocolRegistry.shared.handler(for: id) else {
            client?.urlProtocol(
                self,
                didFailWithError: NSError(
                    domain: "MockURLProtocol",
                    code: -1,
                    userInfo: [NSLocalizedDescriptionKey: "Missing handler for \(id)"]
                )
            )
            return
        }
        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

private final class AtomicCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0

    func incrementAndGet() -> Int {
        lock.lock()
        defer { lock.unlock() }
        value += 1
        return value
    }

    func get() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

final class TokenVaneTests: XCTestCase {
    private func makeTempDir() throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tokenvane-ios-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeMockSession(id: String) -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        config.httpAdditionalHeaders = ["X-Mock-Id": id]
        return URLSession(configuration: config)
    }

    func testPlaintextAndE2EEIngestRollupSpeedAndSnapshotIsolation() async throws {
        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let dbURL = tempDir.appendingPathComponent("test.sqlite")
        let snapshotURL = tempDir.appendingPathComponent("Snapshots/widget-dashboard.json")
        let actor = try CoreClientActor(dbPath: dbURL.path, localDeviceName: "iPhone-Test", snapshotURL: snapshotURL)

        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let hourMs: Int64 = 3_600_000
        let bucketStart = (nowMs / hourMs) * hourMs

        let innerEnvelope: [String: Any] = [
            "schema_version": 1,
            "device_id": "macbook-air-e2ee",
            "device_display_name": "MacBook-Air",
            "generated_at_ms": nowMs,
            "since_watermark_ms": 0,
            "hourly_deltas": [
                [
                    "provider_id": "claude_code",
                    "model_id": "claude-sonnet-4-5",
                    "bucket_start_ms": bucketStart,
                    "request_count": 4,
                    "observable_step_count": 0,
                    "input_tokens": 20_000,
                    "output_tokens": 5_000,
                    "cache_read_tokens": 10_000,
                    "cache_write_tokens": 0,
                    "reasoning_tokens": 0,
                    "estimated_cost_micro_usd": 240_000,
                    "cache_saved_micro_usd": 45_000,
                    "total_duration_ms": 12_000,
                    "tps_sample_count": 4,
                    "tps_sum": 260.0,
                    "max_output_tps": 74.0,
                    "updated_at_ms": nowMs
                ]
            ],
            "subscription_snapshots": [
                [
                    "plan_id": "claude_max_5x",
                    "provider_id": "claude_code",
                    "plan_name": "Claude Max 5x",
                    "window_id": "5h",
                    "window_label": "5h",
                    "used_percent": 55.5,
                    "resets_at_ms": nowMs + 7_200_000,
                    "source": "local_estimation"
                ]
            ]
        ]

        let passphrase = "ios-cross-platform-secret"
        let salt = Data((0..<16).map { UInt8($0 + 1) })
        let nonce = Data((0..<12).map { UInt8(100 + $0) })
        let seqMs: Int64 = 808
        let encryptedEnvelope = try encryptEnvelopeForTest(
            innerEnvelope: innerEnvelope,
            deviceId: "macbook-air-e2ee",
            seqMs: seqMs,
            passphrase: passphrase,
            salt: salt,
            nonce: nonce
        )

        let pullObj: [String: Any] = [
            "ok": true,
            "server_seq_ms": seqMs,
            "next_since_ms": seqMs + 1,
            "has_more": false,
            "items": [
                [
                    "mode": "e2ee",
                    "seq_ms": seqMs,
                    "device_id": "macbook-air-e2ee",
                    "encrypted_envelope": encryptedEnvelope
                ]
            ]
        ]
        let pullData = try JSONSerialization.data(withJSONObject: pullObj)
        let pullJSON = String(data: pullData, encoding: .utf8)!

        // Wrong passphrase must fail cleanly without advancing watermark
        do {
            _ = try await actor.ingestPullResponse(bodyJSON: pullJSON, e2eePassphrase: "wrong-passphrase")
            XCTFail("Expected wrong passphrase error")
        } catch {
            // Expected error
        }
        let bootAfterFail = try await actor.getBootstrap()
        XCTAssertEqual(bootAfterFail.syncConfig.lastPullWatermarkMs, 0)

        // Correct passphrase ingests atomically
        let outcome = try await actor.ingestPullResponse(bodyJSON: pullJSON, e2eePassphrase: passphrase)
        XCTAssertEqual(outcome.envelopesApplied, 1)
        XCTAssertEqual(outcome.hourlyBucketsApplied, 1)
        XCTAssertEqual(outcome.nextSinceMs, seqMs + 1)

        // Speed query must fallback to hourly rollups with fromHourlyRollups = true
        let filter = AnalyticsFilterDTO(startMs: bucketStart - hourMs, endMs: nowMs + hourMs)
        let speed = try await actor.querySpeedLatency(filter: filter)
        XCTAssertTrue(speed.fromHourlyRollups)
        XCTAssertEqual(speed.providers.count, 1)
        XCTAssertEqual(speed.providers.first?.meanTps ?? 0, 65.0, accuracy: 0.01)

        // Atomic widget snapshot must exist, include dayKey, and stay well below 8 KB
        XCTAssertTrue(FileManager.default.fileExists(atPath: snapshotURL.path))
        let snapData = try Data(contentsOf: snapshotURL)
        XCTAssertLessThan(snapData.count, 8192)
        let loaded = AppGroupPaths.loadWidgetSnapshot(from: snapshotURL)
        XCTAssertEqual(loaded?.todayTokens, 35_000)
        XCTAssertEqual(loaded?.primaryQuotas.count, 1)
        XCTAssertFalse(loaded?.dayKey?.isEmpty ?? true)
    }

    func testSSEFrameParserChunkedStream() {
        var parser = SSEFrameParser()
        let chunk1 = Data(": heartbeat\n\nevent: sync_up".utf8)
        let chunk2 = Data("date\ndata: {\"seq_ms\":123".utf8)
        let chunk3 = Data(",\"device_id\":\"win\"}\r\n\r\n".utf8)

        XCTAssertTrue(parser.feed(data: chunk1).isEmpty)
        XCTAssertTrue(parser.feed(data: chunk2).isEmpty)
        let frames = parser.feed(data: chunk3)
        XCTAssertEqual(frames.count, 1)
        XCTAssertEqual(frames.first?.event, "sync_update")
        XCTAssertEqual(frames.first?.data, "{\"seq_ms\":123,\"device_id\":\"win\"}")
    }

    func testPairingPayloadAndSecretStore() throws {
        let raw = "{\"v\":1,\"url\":\"https://sync.example.workers.dev/\",\"token\":\"secret-tok\",\"e2ee\":true}"
        let parsed = try DesktopPairingPayload.parse(raw)
        XCTAssertEqual(parsed.url, "https://sync.example.workers.dev")
        XCTAssertEqual(parsed.token, "secret-tok")
        XCTAssertTrue(parsed.e2ee)
        XCTAssertNil(parsed.key)

        let deepLink = "tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev%2F&token=secret-tok&e2ee=1&key=my-pass"
        let parsedLink = try DesktopPairingPayload.parse(deepLink)
        XCTAssertEqual(parsedLink.url, "https://sync.example.workers.dev")
        XCTAssertEqual(parsedLink.token, "secret-tok")
        XCTAssertTrue(parsedLink.e2ee)
        XCTAssertEqual(parsedLink.key, "my-pass")

        XCTAssertThrowsError(try DesktopPairingPayload.parse("{\"v\":2,\"url\":\"https://x.com\",\"token\":\"t\",\"e2ee\":false}"))
        XCTAssertThrowsError(try DesktopPairingPayload.parse("{\"v\":1,\"url\":\"ftp://x.com\",\"token\":\"t\",\"e2ee\":false}"))

        let store = InMemorySecretStore()
        try store.saveSecret("tok-123", account: .syncBearerToken)
        XCTAssertEqual(try store.loadSecret(account: .syncBearerToken), "tok-123")
        try store.deleteSecret(account: .syncBearerToken)
        XCTAssertNil(try store.loadSecret(account: .syncBearerToken))
    }

    @MainActor
    func testSyncCoordinatorSingleFlightCoalescesConcurrentPullsAndFollowUp() async throws {
        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let actor = try CoreClientActor(
            dbPath: tempDir.appendingPathComponent("sync-sf.sqlite").path,
            localDeviceName: "iPhone-SF",
            snapshotURL: tempDir.appendingPathComponent("snap.json")
        )
        _ = try await actor.updateSyncConfig(
            enabled: true,
            workerURL: "https://mock-sf.tokenvane.local",
            e2eeEnabled: false
        )
        let secrets = InMemorySecretStore(initial: [.syncBearerToken: "tok-sf"])
        let mockId = "sf-\(UUID().uuidString)"
        let counter = AtomicCounter()

        let firstGate = DispatchSemaphore(value: 0)
        MockProtocolRegistry.shared.setHandler(id: mockId) { req in
            let count = counter.incrementAndGet()
            if count == 1 {
                // Hold the first request briefly until concurrent callers enqueue a follow-up.
                _ = firstGate.wait(timeout: .now() + 2.0)
            }
            let seq = Int64(count * 100)
            let payload: [String: Any] = [
                "ok": true,
                "server_seq_ms": seq,
                "next_since_ms": seq + 1,
                "has_more": false,
                "items": []
            ]
            let data = try JSONSerialization.data(withJSONObject: payload)
            let resp = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (resp, data)
        }
        defer { MockProtocolRegistry.shared.setHandler(id: mockId, handler: nil) }

        let coordinator = SyncCoordinator(core: actor, secrets: secrets, session: makeMockSession(id: mockId))

        async let pull1 = coordinator.pullIncremental()
        try await Task.sleep(for: .milliseconds(40))
        async let pull2 = coordinator.pullIncremental()
        async let pull3 = coordinator.pullIncremental()
        try await Task.sleep(for: .milliseconds(25))
        firstGate.signal()

        let results = await (pull1, pull2, pull3)
        XCTAssertTrue(results.0)
        XCTAssertTrue(results.1)
        XCTAssertTrue(results.2)
        // First pull + exactly one coalesced follow-up pass.
        XCTAssertEqual(counter.get(), 2)
        let boot = try await actor.getBootstrap()
        XCTAssertEqual(boot.syncConfig.lastPullWatermarkMs, 201)
    }

    @MainActor
    func testSyncCoordinatorPullFailureSetsErrorAndRecoversOnRetry() async throws {
        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let actor = try CoreClientActor(
            dbPath: tempDir.appendingPathComponent("sync-retry.sqlite").path,
            localDeviceName: "iPhone-Retry",
            snapshotURL: tempDir.appendingPathComponent("snap.json")
        )
        _ = try await actor.updateSyncConfig(
            enabled: true,
            workerURL: "https://mock-retry.tokenvane.local",
            e2eeEnabled: false
        )
        let secrets = InMemorySecretStore(initial: [.syncBearerToken: "tok-retry"])
        let mockId = "retry-\(UUID().uuidString)"
        let counter = AtomicCounter()

        MockProtocolRegistry.shared.setHandler(id: mockId) { req in
            let attempt = counter.incrementAndGet()
            if attempt == 1 {
                let resp = HTTPURLResponse(url: req.url!, statusCode: 502, httpVersion: nil, headerFields: nil)!
                return (resp, Data("Bad Gateway".utf8))
            }
            let payload: [String: Any] = [
                "ok": true,
                "server_seq_ms": 900,
                "next_since_ms": 901,
                "has_more": false,
                "items": []
            ]
            let data = try JSONSerialization.data(withJSONObject: payload)
            let resp = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (resp, data)
        }
        defer { MockProtocolRegistry.shared.setHandler(id: mockId, handler: nil) }

        let coordinator = SyncCoordinator(core: actor, secrets: secrets, session: makeMockSession(id: mockId))
        var states: [SyncConnectionState] = []
        coordinator.onStateChanged = { state in
            states.append(state)
        }

        let firstOk = await coordinator.pullIncremental()
        XCTAssertFalse(firstOk)
        XCTAssertEqual(states.last, .error("Worker HTTP 502"))
        let bootAfterFail = try await actor.getBootstrap()
        XCTAssertEqual(bootAfterFail.syncConfig.lastPullWatermarkMs, 0)

        let secondOk = await coordinator.pullIncremental()
        XCTAssertTrue(secondOk)
        XCTAssertEqual(states.last, .offline)
        let bootAfterRetry = try await actor.getBootstrap()
        XCTAssertEqual(bootAfterRetry.syncConfig.lastPullWatermarkMs, 901)
    }

    func testWidgetSnapshotMidnightRolloverAndExpiredQuotaWindow() throws {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(secondsFromGMT: 8 * 3600)!

        let beforeMidnight = cal.date(from: DateComponents(year: 2026, month: 11, day: 18, hour: 23, minute: 50))!
        let sameDayLater = cal.date(from: DateComponents(year: 2026, month: 11, day: 18, hour: 23, minute: 52))!
        let nextDay = cal.date(from: DateComponents(year: 2026, month: 11, day: 19, hour: 0, minute: 5))!
        let nextMonth = cal.date(from: DateComponents(year: 2026, month: 12, day: 1, hour: 0, minute: 5))!

        let quotaResetBeforeNextDay = cal.date(from: DateComponents(year: 2026, month: 11, day: 18, hour: 23, minute: 55))!
        let quotaResetFuture = cal.date(from: DateComponents(year: 2026, month: 11, day: 19, hour: 4, minute: 0))!

        let snapshot = WidgetDashboardSnapshot(
            generatedAtMs: Int64(beforeMidnight.timeIntervalSince1970 * 1000),
            dayKey: "2026-11-18",
            currency: "USD",
            usdToCnyRate: "7.25",
            todayTokens: 50_000,
            todayCostMicroUsd: 1_200_000,
            todayRequests: 12,
            monthTokens: 500_000,
            monthCostMicroUsd: 12_000_000,
            activeDeviceCount: 1,
            latestOutputTps: 72.0,
            lastSyncAtMs: Int64(beforeMidnight.timeIntervalSince1970 * 1000),
            topProviders: [
                WidgetProviderShare(
                    providerId: "claude_code",
                    displayName: "Claude Code",
                    tokens: 50_000,
                    costMicroUsd: 1_200_000,
                    sharePercent: 100.0
                )
            ],
            primaryQuotas: [
                WidgetQuotaWindowItem(
                    deviceId: "ws",
                    deviceDisplayName: "WS",
                    planId: "claude_max",
                    providerId: "claude_code",
                    planName: "Claude Max",
                    windowId: "5h",
                    windowLabel: "5h",
                    usedPercent: 82.0,
                    resetsAtMs: Int64(quotaResetBeforeNextDay.timeIntervalSince1970 * 1000),
                    updatedAtMs: Int64(beforeMidnight.timeIntervalSince1970 * 1000)
                ),
                WidgetQuotaWindowItem(
                    deviceId: "ws",
                    deviceDisplayName: "WS",
                    planId: "codex_pro",
                    providerId: "codex",
                    planName: "ChatGPT Pro",
                    windowId: "7d",
                    windowLabel: "7d",
                    usedPercent: 40.0,
                    resetsAtMs: Int64(quotaResetFuture.timeIntervalSince1970 * 1000),
                    updatedAtMs: Int64(beforeMidnight.timeIntervalSince1970 * 1000)
                )
            ]
        )

        let sameDayDisplay = snapshot.effectiveForDisplay(at: sameDayLater, calendar: cal)
        XCTAssertEqual(sameDayDisplay.todayTokens, 50_000)
        XCTAssertEqual(sameDayDisplay.todayCostMicroUsd, 1_200_000)
        XCTAssertEqual(sameDayDisplay.topProviders.count, 1)
        XCTAssertEqual(sameDayDisplay.primaryQuotas[0].usedPercent, 82.0)

        let nextDayDisplay = snapshot.effectiveForDisplay(at: nextDay, calendar: cal)
        XCTAssertEqual(nextDayDisplay.todayTokens, 0)
        XCTAssertEqual(nextDayDisplay.todayCostMicroUsd, 0)
        XCTAssertEqual(nextDayDisplay.todayRequests, 0)
        XCTAssertTrue(nextDayDisplay.topProviders.isEmpty)
        XCTAssertEqual(nextDayDisplay.monthTokens, 500_000)
        XCTAssertEqual(nextDayDisplay.monthCostMicroUsd, 12_000_000)
        XCTAssertEqual(nextDayDisplay.primaryQuotas[0].usedPercent, 0.0)
        XCTAssertNil(nextDayDisplay.primaryQuotas[0].resetsAtMs)
        XCTAssertEqual(nextDayDisplay.primaryQuotas[1].usedPercent, 40.0)

        let nextMonthDisplay = snapshot.effectiveForDisplay(at: nextMonth, calendar: cal)
        XCTAssertEqual(nextMonthDisplay.todayTokens, 0)
        XCTAssertEqual(nextMonthDisplay.monthTokens, 0)
        XCTAssertEqual(nextMonthDisplay.monthCostMicroUsd, 0)
    }

    func testQuotaResetDetectorScheduledAndBonusResetsWithoutFalsePositives() {
        func makeSub(used: Double, resetsAt: Int64?, updatedAt: Int64) -> RemoteSubscriptionSnapshotDTO {
            RemoteSubscriptionSnapshotDTO(
                deviceId: "dev-1",
                deviceDisplayName: "Workstation",
                planId: "claude_max",
                windowId: "5h",
                providerId: "claude_code",
                planName: "Claude Max",
                windowLabel: "5h",
                usedPercent: used,
                resetsAtMs: resetsAt,
                source: "official_header",
                updatedAtMs: updatedAt
            )
        }

        // Cold start establishes baseline only
        let r1 = QuotaResetDetector.detectResets(
            previous: [:],
            current: [makeSub(used: 78.0, resetsAt: 10_000, updatedAt: 1_000)],
            nowMs: 1_000
        )
        XCTAssertTrue(r1.events.isEmpty)

        // Normal jitter / climb should not alert
        let r2 = QuotaResetDetector.detectResets(
            previous: r1.baselines,
            current: [makeSub(used: 76.0, resetsAt: 10_000, updatedAt: 2_000)],
            nowMs: 2_000
        )
        XCTAssertTrue(r2.events.isEmpty)

        let r3 = QuotaResetDetector.detectResets(
            previous: r2.baselines,
            current: [makeSub(used: 84.0, resetsAt: 10_000, updatedAt: 3_000)],
            nowMs: 3_000
        )
        XCTAssertTrue(r3.events.isEmpty)

        // Bonus grant: 84% -> 35%
        let r4 = QuotaResetDetector.detectResets(
            previous: r3.baselines,
            current: [makeSub(used: 35.0, resetsAt: 10_000, updatedAt: 4_000)],
            nowMs: 4_000
        )
        XCTAssertEqual(r4.events.count, 1)
        XCTAssertEqual(r4.events.first?.previousPercent, 84.0)
        XCTAssertEqual(r4.events.first?.currentPercent, 35.0)

        // Re-evaluating the same 35% snapshot does not duplicate
        let r5 = QuotaResetDetector.detectResets(
            previous: r4.baselines,
            current: [makeSub(used: 35.0, resetsAt: 10_000, updatedAt: 4_500)],
            nowMs: 4_500
        )
        XCTAssertTrue(r5.events.isEmpty)

        // Scheduled reset: 35% -> 0%
        let r6 = QuotaResetDetector.detectResets(
            previous: r5.baselines,
            current: [makeSub(used: 0.0, resetsAt: 28_000, updatedAt: 5_000)],
            nowMs: 5_000
        )
        XCTAssertEqual(r6.events.count, 1)
        XCTAssertEqual(r6.events.first?.previousPercent, 35.0)
        XCTAssertEqual(r6.events.first?.currentPercent, 0.0)

        // Repeated 0% is ignored
        let r7 = QuotaResetDetector.detectResets(
            previous: r6.baselines,
            current: [makeSub(used: 0.0, resetsAt: 28_000, updatedAt: 6_000)],
            nowMs: 6_000
        )
        XCTAssertTrue(r7.events.isEmpty)

        // Small usage (8%) followed by full zeroing also triggers
        let r8 = QuotaResetDetector.detectResets(
            previous: r7.baselines,
            current: [makeSub(used: 8.0, resetsAt: 28_000, updatedAt: 7_000)],
            nowMs: 7_000
        )
        XCTAssertTrue(r8.events.isEmpty)

        let r9 = QuotaResetDetector.detectResets(
            previous: r8.baselines,
            current: [makeSub(used: 0.0, resetsAt: 46_000, updatedAt: 8_000)],
            nowMs: 8_000
        )
        XCTAssertEqual(r9.events.count, 1)
    }

    @MainActor
    func testCurrencyThousandsGroupingRateValidationAndExchangeRateThrottle() async throws {
        XCTAssertEqual(
            TokenVaneFormatters.formatCurrency(1_234_500_000, currency: "USD", rateString: "7.25"),
            "$1,234.5"
        )
        XCTAssertEqual(
            TokenVaneFormatters.formatCurrency(-1_234_500_000, currency: "USD", rateString: "7.25"),
            "-$1,234.5"
        )
        XCTAssertEqual(
            TokenVaneFormatters.formatCurrency(1_234_500_000, currency: "CNY", rateString: "7.25"),
            "¥8,950.1"
        )

        XCTAssertEqual(TokenVaneFormatters.validateExchangeRate("7.25"), 7.25)
        XCTAssertNil(TokenVaneFormatters.validateExchangeRate("0.05"))
        XCTAssertNil(TokenVaneFormatters.validateExchangeRate("150"))
        XCTAssertNil(TokenVaneFormatters.validateExchangeRate("abc"))

        var utcCal = Calendar(identifier: .gregorian)
        utcCal.timeZone = TimeZone(secondsFromGMT: 0)!
        let sampleDate = utcCal.date(from: DateComponents(year: 2026, month: 11, day: 18, hour: 8, minute: 30, second: 5))!
        let sampleMs = Int64(sampleDate.timeIntervalSince1970 * 1000)
        XCTAssertEqual(
            TokenVaneFormatters.formatWatermarkTimestamp(sampleMs, timeZone: utcCal.timeZone),
            "2026-11-18 08:30:05"
        )
        XCTAssertEqual(
            TokenVaneFormatters.formatWatermarkTimestamp(nil),
            L10n.tr("sync.status.offline", default: "待同步")
        )

        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let actor = try CoreClientActor(
            dbPath: tempDir.appendingPathComponent("fx-throttle.sqlite").path,
            localDeviceName: "iPhone-FX",
            snapshotURL: tempDir.appendingPathComponent("snap.json")
        )
        let mockId = "fx-\(UUID().uuidString)"
        let counter = AtomicCounter()
        MockProtocolRegistry.shared.setHandler(id: mockId) { req in
            _ = counter.incrementAndGet()
            let json = Data(#"{"amount":1.0,"base":"USD","rates":{"CNY":7.2888}}"#.utf8)
            let resp = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (resp, json)
        }
        defer { MockProtocolRegistry.shared.setHandler(id: mockId, handler: nil) }

        let coordinator = SyncCoordinator(
            core: actor,
            secrets: InMemorySecretStore(),
            session: makeMockSession(id: mockId)
        )

        let baseNowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let didFetch1 = await coordinator.syncExchangeRateIfAuto(nowMs: baseNowMs)
        XCTAssertTrue(didFetch1)
        XCTAssertEqual(counter.get(), 1)

        // Within 12h window -> throttled
        let didFetch2 = await coordinator.syncExchangeRateIfAuto(nowMs: baseNowMs + 3_600_000)
        XCTAssertFalse(didFetch2)
        XCTAssertEqual(counter.get(), 1)

        // After 12h -> fetches again
        let didFetch3 = await coordinator.syncExchangeRateIfAuto(nowMs: baseNowMs + SyncCoordinator.exchangeRateThrottleMs + 60_000)
        XCTAssertTrue(didFetch3)
        XCTAssertEqual(counter.get(), 2)
    }

    @MainActor
    func testAppStoreTargetedTrendsReloadAndPairingConfirmation() async throws {
        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let suiteName = "tokenvane.test.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let actor = try CoreClientActor(
            dbPath: tempDir.appendingPathComponent("store.sqlite").path,
            localDeviceName: "iPhone-Store",
            snapshotURL: tempDir.appendingPathComponent("snap.json")
        )
        let secrets = InMemorySecretStore()
        let coordinator = SyncCoordinator(core: actor, secrets: secrets, session: makeMockSession(id: "unused"))
        let notifier = QuotaResetNotifier(userDefaults: defaults, installCenterDelegate: false)
        let store = AppStore(
            core: actor,
            secrets: secrets,
            syncCoordinator: coordinator,
            resetNotifier: notifier,
            userDefaults: defaults,
            demoMode: true,
            stageDemoPairing: false
        )

        await store.bootstrapAndSeedIfNeeded()

        // Pairing confirmation flow: cancel keeps prior state; confirm writes config & token.
        XCTAssertTrue(store.stagePairing(raw: "tokenvane://pair?v=1&url=https%3A%2F%2Fnew-worker.tokenvane.local&token=pair-tok&e2ee=0"))
        XCTAssertEqual(store.pendingPairing?.url, "https://new-worker.tokenvane.local")
        store.cancelPendingPairing()
        XCTAssertNil(store.pendingPairing)
        XCTAssertEqual(store.bootstrap?.syncConfig.workerUrl, "https://sync-demo.tokenvane.local")

        XCTAssertTrue(store.stagePairing(raw: "tokenvane://pair?v=1&url=https%3A%2F%2Fnew-worker.tokenvane.local&token=pair-tok&e2ee=0"))
        if let pending = store.pendingPairing {
            await store.confirmPairing(pending)
        }
        XCTAssertNil(store.pendingPairing)
        XCTAssertEqual(store.bootstrap?.syncConfig.workerUrl, "https://new-worker.tokenvane.local")
        XCTAssertEqual(store.loadSecretMasked(account: .syncBearerToken), "pair-tok")
        XCTAssertGreaterThan(store.syncConfigRevision, 0)

        // Targeted trends reload updates trends without invalidating overview
        let beforeOverview = store.overview
        await store.selectTrendDimension(.model)
        await store.selectTrendGranularity(.daily)
        XCTAssertEqual(store.overview, beforeOverview)
        XCTAssertNotNil(store.trends)
        XCTAssertEqual(store.trends?.effectiveGranularity, .daily)

        // Manual rate preservation across auto FX toggle
        let invalidApplied = await store.applyManualExchangeRate("bad")
        XCTAssertFalse(invalidApplied)
        let validApplied = await store.applyManualExchangeRate("7.35")
        XCTAssertTrue(validApplied)
        XCTAssertEqual(store.manualRateDraft, "7.35")

        await store.setAutoExchangeRate(true)
        XCTAssertEqual(store.manualRateDraft, "7.35")
        await store.setAutoExchangeRate(false)
        XCTAssertEqual(store.usdToCnyRate, "7.35")
    }

    func testModelComparisonBuilderAndReceiptModelBuilderAndHeatmap() {
        let pricing = ModelPricingEntryDTO(
            modelPattern: "claude-sonnet-4-5",
            providerId: "claude_code",
            inputPerMMicroUsd: 3_000_000,
            outputPerMMicroUsd: 15_000_000,
            cacheReadPerMMicroUsd: 300_000,
            cacheWritePerMMicroUsd: 3_750_000,
            reasoningPerMMicroUsd: 15_000_000,
            isUserOverride: false,
            effectiveFrom: "2026-11-18"
        )
        let modelStats = ModelStatsReportDTO(
            rows: [
                ModelStatRowDTO(
                    modelId: "claude-sonnet-4-5",
                    providerId: "claude_code",
                    requests: 10,
                    observableSteps: 10,
                    inputTokens: 100_000,
                    outputTokens: 50_000,
                    cacheReadTokens: 300_000,
                    cacheWriteTokens: 0,
                    reasoningTokens: 0,
                    totalTokens: 450_000,
                    cacheSavedMicroUsd: 500_000,
                    estimatedCostMicroUsd: 1_140_000,
                    costShareRatio: 1.0,
                    avgOutputTps: 60.0,
                    effectivePricing: pricing
                )
            ],
            totalCostMicroUsd: 1_140_000,
            totalCacheSavedMicroUsd: 500_000,
            catalog: [pricing]
        )
        let speedReport = SpeedLatencyReportDTO(
            providers: [
                ProviderSpeedStatsDTO(
                    providerId: "claude_code",
                    sampleCount: 10,
                    meanTps: 62.5,
                    p50Tps: 61.0,
                    p95Tps: 78.0,
                    maxTps: 84.0,
                    minTps: 42.0,
                    avgDurationMs: 2_100.0,
                    avgTtftMs: 410.0
                )
            ],
            models: [
                ModelSpeedStatsDTO(
                    modelId: "claude-sonnet-4-5",
                    providerId: "claude_code",
                    sampleCount: 10,
                    meanTps: 62.5,
                    p50Tps: 61.0,
                    p95Tps: 78.0,
                    maxTps: 84.0,
                    minTps: 42.0,
                    latestTps: 64.0,
                    avgDurationMs: 2_100.0,
                    avgTtftMs: 410.0
                ),
                ModelSpeedStatsDTO(
                    modelId: "gpt-5.3-codex",
                    providerId: "codex",
                    sampleCount: 3,
                    meanTps: 80.0,
                    p50Tps: 80.0,
                    p95Tps: 85.0,
                    maxTps: 88.0,
                    minTps: 70.0,
                    latestTps: 81.0,
                    avgDurationMs: 1_500.0,
                    avgTtftMs: nil
                )
            ],
            fromHourlyRollups: false
        )

        let builtModels = ModelComparisonBuilder.buildModelComparison(models: modelStats, speed: speedReport)
        XCTAssertEqual(builtModels.count, 2)
        XCTAssertEqual(builtModels[0].modelId, "claude-sonnet-4-5")
        XCTAssertEqual(builtModels[0].outputPricePerMMicroUsd, 15_000_000)
        XCTAssertEqual(builtModels[0].cacheHitRatio ?? 0, 0.75, accuracy: 0.001)
        XCTAssertEqual(builtModels[1].modelId, "gpt-5.3-codex")
        XCTAssertNil(builtModels[1].costPerMMicroUsd)

        let sortedAsc = ModelComparisonBuilder.sortComparisonRows(builtModels, key: .costPerMMicroUsd, descending: false)
        XCTAssertEqual(sortedAsc.last?.modelId, "gpt-5.3-codex")
        let sortedDesc = ModelComparisonBuilder.sortComparisonRows(builtModels, key: .costPerMMicroUsd, descending: true)
        XCTAssertEqual(sortedDesc.last?.modelId, "gpt-5.3-codex")

        let providers = ModelComparisonBuilder.buildProviderComparison(rows: builtModels, speed: speedReport, models: modelStats)
        XCTAssertEqual(providers.count, 2)
        XCTAssertEqual(providers.first?.providerId, "claude_code")
        XCTAssertEqual(providers.first?.costShare ?? 0, 1.0, accuracy: 0.001)

        let shares: [UsageShareItemDTO] = (1...10).map { idx in
            UsageShareItemDTO(
                key: "model-\(idx)",
                label: "Model \(idx)",
                tokens: Int64(idx * 10_000),
                tokenSharePercent: 10.0,
                costMicroUsd: Int64(idx * 1_000_000),
                costSharePercent: 10.0
            )
        }
        let report = MonthlyReportDTO(
            year: 2026,
            month: 11,
            apiEquivalentCostMicroUsd: 55_000_000,
            subscriptionMonthlyFeeMicroUsd: 20_000_000,
            roiMultiplier: 2.75,
            monthTotalTokens: 550_000,
            monthTotalRequests: 42,
            monthCacheSavedMicroUsd: 3_000_000,
            providerShares: [],
            modelShares: shares,
            dailyLedger: [
                MonthlyDailyLedgerRowDTO(
                    date: "2026-11-17",
                    requests: 20,
                    observableSteps: 20,
                    inputTokens: 100_000,
                    outputTokens: 20_000,
                    cacheReadTokens: 80_000,
                    cacheWriteTokens: 0,
                    reasoningTokens: 0,
                    totalTokens: 200_000,
                    costMicroUsd: 20_000_000,
                    cacheSavedMicroUsd: 1_000_000,
                    topModel: "model-10"
                ),
                MonthlyDailyLedgerRowDTO(
                    date: "2026-11-18",
                    requests: 22,
                    observableSteps: 22,
                    inputTokens: 150_000,
                    outputTokens: 30_000,
                    cacheReadTokens: 170_000,
                    cacheWriteTokens: 0,
                    reasoningTokens: 0,
                    totalTokens: 350_000,
                    costMicroUsd: 35_000_000,
                    cacheSavedMicroUsd: 2_000_000,
                    topModel: "model-10"
                )
            ],
            heatmap365d: [
                HeatmapDayCellDTO(date: "2026-11-15", totalTokens: 0, costMicroUsd: 0, requests: 0, topModel: nil),
                HeatmapDayCellDTO(date: "2026-11-16", totalTokens: 1_000, costMicroUsd: 100, requests: 1, topModel: nil),
                HeatmapDayCellDTO(date: "2026-11-17", totalTokens: 200_000, costMicroUsd: 20_000_000, requests: 20, topModel: nil),
                HeatmapDayCellDTO(date: "2026-11-18", totalTokens: 350_000, costMicroUsd: 35_000_000, requests: 22, topModel: nil)
            ],
            rangeStartMs: nil,
            rangeEndMs: nil,
            subscriptionPeriodFeeMicroUsd: 20_000_000
        )

        let receipt = ReceiptModelBuilder.buildReceiptModel(
            report: report,
            currency: "USD",
            usdToCnyRate: "7.25",
            periodLabel: "2026-11"
        )
        XCTAssertEqual(receipt.fileName, "tokenvane-receipt-2026-11.png")
        XCTAssertTrue(receipt.lines.contains {
            if case .row(let left, let right, _) = $0 {
                return left == L10n.tr("receipt.order_no", default: "单号") && right.hasPrefix("TV202611-")
            }
            return false
        })
        XCTAssertTrue(receipt.lines.contains {
            if case .item(let left, _, _) = $0 {
                return left.contains("2")
            }
            return false
        })
        XCTAssertEqual(ReceiptModelBuilder.longestActiveStreak(report: report), 2)

        let levels = ReceiptModelBuilder.computeHeatmapIntensityLevels(report.heatmap365d)
        XCTAssertEqual(levels["2026-11-15"], 0)
        XCTAssertEqual(levels["2026-11-18"], 4)
    }

    func testPairingURIGeneratorAndExtendedKeychainAccounts() throws {
        let uri = DesktopPairingPayload.makePairingURI(
            url: "https://sync.example.workers.dev/",
            token: "tok-abc",
            e2ee: true,
            key: "pass-xyz"
        )
        let uriString = try XCTUnwrap(uri)
        let reparsed = try DesktopPairingPayload.parse(uriString)
        XCTAssertEqual(reparsed.url, "https://sync.example.workers.dev")
        XCTAssertEqual(reparsed.token, "tok-abc")
        XCTAssertTrue(reparsed.e2ee)
        XCTAssertEqual(reparsed.key, "pass-xyz")

        let store = InMemorySecretStore()
        try store.saveSecret("cf-secret", account: .cloudflareApiToken)
        try store.saveSecret("ds-key", account: .providerApiKey("acct-1"))
        XCTAssertEqual(try store.loadSecret(account: .cloudflareApiToken), "cf-secret")
        XCTAssertEqual(try store.loadSecret(account: .providerApiKey("acct-1")), "ds-key")
    }

    func testCoreClientNewFFIMethodsRoundTrip() async throws {
        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let actor = try CoreClientActor(
            dbPath: tempDir.appendingPathComponent("ffi-parity.sqlite").path,
            localDeviceName: "iPhone-FFI",
            snapshotURL: tempDir.appendingPathComponent("snap.json")
        )

        let pricingEntry = ModelPricingEntryDTO(
            modelPattern: "custom-fast-model",
            providerId: "deepseek_harness",
            inputPerMMicroUsd: 1_000_000,
            outputPerMMicroUsd: 2_000_000,
            cacheReadPerMMicroUsd: 100_000,
            cacheWritePerMMicroUsd: 0,
            reasoningPerMMicroUsd: 2_000_000,
            isUserOverride: true,
            effectiveFrom: "2026-11-18"
        )
        try await actor.upsertModelPricing(pricingEntry)
        let afterUpsert = try await actor.queryModelsPricing(filter: AnalyticsFilterDTO(startMs: 0, endMs: 9_999_999_999_999, allTime: true))
        XCTAssertTrue(afterUpsert.catalog.contains { $0.modelPattern == "custom-fast-model" && $0.isUserOverride })
        try await actor.deleteModelPricingOverride(modelPattern: "custom-fast-model")

        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let account = PaygAccountDTO(
            accountId: "payg-ds-1",
            providerId: "deepseek_harness",
            accountName: "DeepSeek Test",
            enabled: true,
            manualBalanceMicroUsd: 5_000_000,
            balanceAnchorMs: nowMs,
            monthlyBudgetMicroUsd: 20_000_000,
            isAuto: false,
            updatedAtMs: nowMs
        )
        try await actor.upsertPaygAccount(account)

        let dsBody = #"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"68.50","granted_balance":"10.00","topped_up_balance":"58.50"}]}"#
        let applied = try await actor.applyOfficialBalancePayload(
            providerId: "deepseek_harness",
            endpoint: "https://api.deepseek.com/user/balance",
            bodyJSON: dsBody,
            nowMs: nowMs
        )
        XCTAssertEqual(applied.balance.currency, "CNY")
        XCTAssertEqual(applied.balance.totalBalanceMicro, 68_500_000)

        let paygList = try await actor.queryPaygAccounts(nowMs: nowMs)
        XCTAssertEqual(paygList.count, 1)
        XCTAssertEqual(paygList.first?.officialBalance?.totalBalanceMicro, 68_500_000)
        try await actor.deletePaygAccount(accountId: "payg-ds-1")
        let afterDeletePayg = try await actor.queryPaygAccounts(nowMs: nowMs)
        XCTAssertTrue(afterDeletePayg.isEmpty)

        let templates = try await actor.listSubscriptionTemplates(nowMs: nowMs)
        XCTAssertFalse(templates.isEmpty)

        let plan = SubscriptionPlanDTO(
            planId: "plan-ios-test",
            providerId: "claude_code",
            planName: "Claude Pro Test",
            enabled: true,
            monthlyFeeMicroUsd: 20_000_000,
            windows: [
                SubscriptionWindowRuleDTO(
                    windowId: "5h",
                    label: "5h 滚动",
                    durationHours: 5,
                    rolling: true,
                    unit: "micro_usd",
                    limitValue: 5_000_000
                )
            ],
            isUserConfigured: true,
            autoSource: nil,
            updatedAtMs: nowMs
        )
        try await actor.upsertSubscriptionPlan(plan)
        let localSubs = try await actor.queryLocalSubscriptions(nowMs: nowMs)
        XCTAssertEqual(localSubs.count, 1)
        try await actor.deleteSubscriptionPlan(planId: "plan-ios-test")
        let afterDeleteSubs = try await actor.queryLocalSubscriptions(nowMs: nowMs)
        XCTAssertTrue(afterDeleteSubs.isEmpty)

        let bundle = try await actor.buildWorkerUploadBundle(
            workerName: "custom-mobile-worker",
            bearerToken: "tok-mobile-1234567890",
            e2eeEnabled: true,
            e2eePassphrase: "pass-mobile-1234",
            includeMigration: true
        )
        XCTAssertEqual(bundle.workerName, "custom-mobile-worker")
        XCTAssertTrue(bundle.contentType.hasPrefix("multipart/form-data; boundary="))
        XCTAssertNotNil(Data(base64Encoded: bundle.multipartBodyBase64))
    }

    @MainActor
    func testSyncCoordinatorCloudflareWorkerProvisioningWithMigrationRetry() async throws {
        let tempDir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let actor = try CoreClientActor(
            dbPath: tempDir.appendingPathComponent("provision.sqlite").path,
            localDeviceName: "iPhone-Provision",
            snapshotURL: tempDir.appendingPathComponent("snap.json")
        )
        let secrets = InMemorySecretStore()
        let mockId = "cf-\(UUID().uuidString)"
        let scriptPutAttempts = AtomicCounter()

        MockProtocolRegistry.shared.setHandler(id: mockId) { req in
            let url = req.url?.absoluteString ?? ""
            let method = req.httpMethod ?? "GET"
            let resp200 = { (obj: Any) -> (HTTPURLResponse, Data) in
                let data = try JSONSerialization.data(withJSONObject: obj)
                return (HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!, data)
            }

            if url.contains("/client/v4/accounts?per_page=5") {
                return try resp200(["success": true, "result": [["id": "acc1234567890"]]])
            }
            if url.hasSuffix("/workers/subdomain") && method == "GET" {
                return try resp200(["success": true, "result": ["subdomain": "myteam"]])
            }
            if url.hasSuffix("/workers/scripts/tokenvane-sync-worker/settings") {
                return (HTTPURLResponse(url: req.url!, statusCode: 404, httpVersion: nil, headerFields: nil)!, Data())
            }
            if url.hasSuffix("/workers/scripts/tokenvane-sync-worker") && method == "PUT" {
                let attempt = scriptPutAttempts.incrementAndGet()
                if attempt == 1 {
                    let errData = Data(#"{"success":false,"errors":[{"code":10079,"message":"migration already applied"}]}"#.utf8)
                    return (HTTPURLResponse(url: req.url!, statusCode: 400, httpVersion: nil, headerFields: nil)!, errData)
                }
                return try resp200(["success": true, "result": ["id": "tokenvane-sync-worker"]])
            }
            if url.hasSuffix("/workers/scripts/tokenvane-sync-worker/subdomain") && method == "POST" {
                return try resp200(["success": true, "result": ["enabled": true]])
            }
            if url.contains("/v1/sync/pull") {
                return try resp200([
                    "ok": true,
                    "server_seq_ms": 10,
                    "next_since_ms": 11,
                    "has_more": false,
                    "items": []
                ])
            }
            return (HTTPURLResponse(url: req.url!, statusCode: 404, httpVersion: nil, headerFields: nil)!, Data())
        }
        defer { MockProtocolRegistry.shared.setHandler(id: mockId, handler: nil) }

        @MainActor final class StepLog {
            var steps: [CloudSyncProvisionStep] = []
        }
        let stepLog = StepLog()
        let coordinator = SyncCoordinator(core: actor, secrets: secrets, session: makeMockSession(id: mockId))
        let outcome = try await coordinator.provisionCloudflareWorker(
            workerName: "tokenvane-sync-worker",
            cfApiToken: "cf-tok-secret",
            cfAccountId: nil,
            bearerToken: "bearer-custom-tok",
            e2eeEnabled: true,
            e2eePassphrase: "pass-custom-e2ee",
            onProgress: { step in
                stepLog.steps.append(step)
            }
        )

        XCTAssertEqual(scriptPutAttempts.get(), 2)
        XCTAssertEqual(outcome.workerURL, "https://tokenvane-sync-worker.myteam.workers.dev")
        XCTAssertTrue(outcome.initialSyncOK)
        XCTAssertEqual(stepLog.steps, CloudSyncProvisionStep.allCases)
        XCTAssertEqual(try secrets.loadSecret(account: .cloudflareApiToken), "cf-tok-secret")
        XCTAssertEqual(try secrets.loadSecret(account: .syncBearerToken), "bearer-custom-tok")
        XCTAssertEqual(try secrets.loadSecret(account: .e2eePassphrase), "pass-custom-e2ee")

        let reparsedPairing = try DesktopPairingPayload.parse(outcome.pairingURI)
        XCTAssertEqual(reparsedPairing.url, "https://tokenvane-sync-worker.myteam.workers.dev")
        XCTAssertEqual(reparsedPairing.token, "bearer-custom-tok")
        XCTAssertEqual(reparsedPairing.key, "pass-custom-e2ee")
    }

    func testParetoFrontierCacheWriteDenominatorAndSpeedBasis() throws {
        // 1. Counts cache writes as prompt tokens in both model & provider cache hit ratios.
        let cacheReport = ModelStatsReportDTO(
            rows: [
                ModelStatRowDTO(
                    modelId: "claude-opus-5-5",
                    providerId: "claude_code",
                    requests: 10,
                    observableSteps: 0,
                    inputTokens: 100,
                    outputTokens: 5_000,
                    cacheReadTokens: 980_000,
                    cacheWriteTokens: 19_900,
                    reasoningTokens: 0,
                    totalTokens: 1_005_000,
                    cacheSavedMicroUsd: 0,
                    estimatedCostMicroUsd: 1_000_000,
                    costShareRatio: 1.0,
                    avgOutputTps: 0
                )
            ],
            totalCostMicroUsd: 1_000_000,
            totalCacheSavedMicroUsd: 0
        )
        let cacheModels = ModelComparisonBuilder.buildModelComparison(models: cacheReport, speed: nil)
        XCTAssertEqual(cacheModels[0].cacheHitRatio ?? 0, 0.98, accuracy: 0.0001)
        let cacheProviders = ModelComparisonBuilder.buildProviderComparison(rows: cacheModels, speed: nil, models: cacheReport)
        XCTAssertEqual(cacheProviders[0].cacheHitRatio ?? 0, 0.98, accuracy: 0.0001)

        // 2. Prices rows across list/actual/subscription and falls back to actual without a paid plan.
        let pricingReport = ModelStatsReportDTO(
            rows: [
                ModelStatRowDTO(
                    modelId: "claude-opus-5-5",
                    providerId: "claude_code",
                    requests: 10,
                    observableSteps: 0,
                    inputTokens: 0,
                    outputTokens: 0,
                    cacheReadTokens: 0,
                    cacheWriteTokens: 0,
                    reasoningTokens: 0,
                    totalTokens: 2_000_000,
                    cacheSavedMicroUsd: 0,
                    estimatedCostMicroUsd: 4_000_000,
                    costShareRatio: 0.8,
                    avgOutputTps: 0,
                    listCostMicroUsd: 10_000_000,
                    subscriptionCostMicroUsd: 1_000_000
                ),
                ModelStatRowDTO(
                    modelId: "k3.1-pro",
                    providerId: "kimi_code",
                    requests: 5,
                    observableSteps: 0,
                    inputTokens: 0,
                    outputTokens: 0,
                    cacheReadTokens: 0,
                    cacheWriteTokens: 0,
                    reasoningTokens: 0,
                    totalTokens: 1_000_000,
                    cacheSavedMicroUsd: 0,
                    estimatedCostMicroUsd: 1_000_000,
                    costShareRatio: 0.2,
                    avgOutputTps: 0,
                    listCostMicroUsd: 3_000_000,
                    subscriptionCostMicroUsd: nil
                )
            ],
            totalCostMicroUsd: 5_000_000,
            totalCacheSavedMicroUsd: 0,
            subscriptionAllocations: [
                ProviderSubscriptionAllocationDTO(
                    providerId: "claude_code",
                    planNames: ["Claude Pro"],
                    monthlyFeeMicroUsd: 20_000_000,
                    periodFeeMicroUsd: 1_000_000
                )
            ]
        )
        let pricedRows = ModelComparisonBuilder.buildModelComparison(models: pricingReport, speed: nil)
        let withPlan = try XCTUnwrap(pricedRows.first { $0.providerId == "claude_code" })
        let withoutPlan = try XCTUnwrap(pricedRows.first { $0.providerId == "kimi_code" })

        XCTAssertEqual(ModelComparisonBuilder.priceForBasis(row: withPlan, basis: .list), BasisPrice(perMMicroUsd: 5_000_000, basis: .list))
        XCTAssertEqual(ModelComparisonBuilder.priceForBasis(row: withPlan, basis: .actual), BasisPrice(perMMicroUsd: 2_000_000, basis: .actual))
        XCTAssertEqual(ModelComparisonBuilder.priceForBasis(row: withPlan, basis: .subscription), BasisPrice(perMMicroUsd: 500_000, basis: .subscription))
        XCTAssertEqual(ModelComparisonBuilder.priceForBasis(row: withoutPlan, basis: .subscription), BasisPrice(perMMicroUsd: 1_000_000, basis: .actual))

        let emptyRow = ModelComparisonRow(
            key: "empty",
            modelId: "empty",
            providerId: "kimi_code",
            requests: 0,
            totalTokens: 0,
            outputTokens: 0,
            costMicroUsd: 0,
            cacheSavedMicroUsd: 0,
            costPerMMicroUsd: nil,
            outputPricePerMMicroUsd: nil,
            cacheHitRatio: nil,
            meanTps: nil,
            p95Tps: nil,
            avgTtftMs: nil,
            avgDurationMs: nil,
            speedSamples: 0
        )
        XCTAssertNil(ModelComparisonBuilder.priceForBasis(row: emptyRow, basis: .actual))

        // 3. Pareto frontier excludes dominated models and keeps exact ties.
        let pareto = ModelComparisonBuilder.computeParetoFrontier([
            FrontierPointInput(key: "cheap-slow", price: 0.2, tps: 20),
            FrontierPointInput(key: "mid", price: 0.5, tps: 80),
            FrontierPointInput(key: "pricey-fast", price: 1.2, tps: 150),
            FrontierPointInput(key: "dominated", price: 0.8, tps: 60),
            FrontierPointInput(key: "twin", price: 0.5, tps: 80)
        ])
        XCTAssertEqual(pareto.frontier, ["cheap-slow", "mid", "twin", "pricey-fast"])
        XCTAssertEqual(pareto.dominatedBy["dominated"], "mid")
        XCTAssertEqual(pareto.dominatedBy.count, 1)

        // 4. Carries speed_basis through DTO decoding and excludes end_to_end & <20 samples from frontier eligibility.
        let speedJSON = """
        {
          "providers": [
            {
              "provider_id": "codex",
              "sample_count": 120,
              "mean_tps": 42.0,
              "p50_tps": 40.0,
              "p95_tps": 70.0,
              "max_tps": 90.0,
              "min_tps": 12.0,
              "avg_duration_ms": 9000.0,
              "avg_ttft_ms": 3000.0,
              "speed_basis": "decode"
            }
          ],
          "models": [
            {
              "model_id": "gpt-6-astra",
              "provider_id": "codex",
              "sample_count": 120,
              "mean_tps": 42.0,
              "p50_tps": 40.0,
              "p95_tps": 70.0,
              "max_tps": 90.0,
              "min_tps": 12.0,
              "latest_tps": 41.0,
              "avg_duration_ms": 9000.0,
              "avg_ttft_ms": 3000.0,
              "speed_basis": "decode"
            },
            {
              "model_id": "gemini-pro",
              "provider_id": "antigravity",
              "sample_count": 120,
              "mean_tps": 30.0,
              "p50_tps": 28.0,
              "p95_tps": 40.0,
              "max_tps": 50.0,
              "min_tps": 15.0,
              "latest_tps": 31.0,
              "avg_duration_ms": 9000.0,
              "avg_ttft_ms": 3000.0,
              "speed_basis": "end_to_end"
            },
            {
              "model_id": "legacy",
              "provider_id": "claude_code",
              "sample_count": 25,
              "mean_tps": 50.0,
              "p50_tps": 50.0,
              "p95_tps": 60.0,
              "max_tps": 70.0,
              "min_tps": 30.0,
              "latest_tps": 50.0,
              "avg_duration_ms": 5000.0,
              "avg_ttft_ms": 500.0
            }
          ],
          "from_hourly_rollups": false
        }
        """
        let decodedSpeed = try JSONDecoder().decode(SpeedLatencyReportDTO.self, from: Data(speedJSON.utf8))
        let basisRows = ModelComparisonBuilder.buildModelComparison(models: nil, speed: decodedSpeed)
        let basisByModel = Dictionary(uniqueKeysWithValues: basisRows.map { ($0.modelId, $0.speedBasis) })
        XCTAssertEqual(basisByModel["gpt-6-astra"] ?? nil, .decode)
        XCTAssertEqual(basisByModel["gemini-pro"] ?? nil, .endToEnd)
        XCTAssertEqual(basisByModel["legacy"] ?? nil, nil)

        let enough = ModelComparisonBuilder.minFrontierSpeedSamples
        XCTAssertTrue(ModelComparisonBuilder.isFrontierEligible(samples: enough, speedBasis: .decode))
        XCTAssertTrue(ModelComparisonBuilder.isFrontierEligible(samples: enough, speedBasis: nil))
        XCTAssertFalse(ModelComparisonBuilder.isFrontierEligible(samples: enough, speedBasis: .endToEnd))
        XCTAssertFalse(ModelComparisonBuilder.isFrontierEligible(samples: enough - 1, speedBasis: .decode))
    }

    private func encryptEnvelopeForTest(
        innerEnvelope: [String: Any],
        deviceId: String,
        seqMs: Int64,
        passphrase: String,
        salt: Data,
        nonce: Data
    ) throws -> [String: Any] {
        let plaintext = try JSONSerialization.data(withJSONObject: innerEnvelope)
        var derivedKey = Data(count: 32)
        let passData = Array(passphrase.utf8).map { Int8(bitPattern: $0) }
        let saltBytes = [UInt8](salt)
        let status = derivedKey.withUnsafeMutableBytes { keyBuf -> Int32 in
            CCKeyDerivationPBKDF(
                CCPBKDFAlgorithm(kCCPBKDF2),
                passData,
                passData.count,
                saltBytes,
                saltBytes.count,
                CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256),
                210_000,
                keyBuf.bindMemory(to: UInt8.self).baseAddress,
                32
            )
        }
        XCTAssertEqual(status, Int32(kCCSuccess))

        let key = SymmetricKey(data: derivedKey)
        let gcmNonce = try AES.GCM.Nonce(data: nonce)
        let aad = Data("v1:\(deviceId):\(seqMs)".utf8)
        let sealed = try AES.GCM.seal(plaintext, using: key, nonce: gcmNonce, authenticating: aad)
        let combinedCiphertext = sealed.ciphertext + sealed.tag

        return [
            "schema_version": 1,
            "device_id": deviceId,
            "seq_ms": seqMs,
            "kdf_salt_hex": salt.map { String(format: "%02x", $0) }.joined(),
            "nonce_hex": nonce.map { String(format: "%02x", $0) }.joined(),
            "ciphertext_b64": combinedCiphertext.base64EncodedString()
        ]
    }
}
