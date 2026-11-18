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
