import CommonCrypto
import CryptoKit
import Foundation
import XCTest
@testable import TokenVane

final class TokenVaneTests: XCTestCase {
    private var tempDir: URL!

    override func setUpWithError() throws {
        tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tokenvane-ios-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let tempDir {
            try? FileManager.default.removeItem(at: tempDir)
        }
    }

    func testPlaintextAndE2EEIngestRollupSpeedAndSnapshotIsolation() async throws {
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
                    "window_label": "5h 滚动窗口",
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

        // Atomic widget snapshot must exist and stay well below 8 KB
        XCTAssertTrue(FileManager.default.fileExists(atPath: snapshotURL.path))
        let snapData = try Data(contentsOf: snapshotURL)
        XCTAssertLessThan(snapData.count, 8192)
        let loaded = AppGroupPaths.loadWidgetSnapshot(from: snapshotURL)
        XCTAssertEqual(loaded?.todayTokens, 35_000)
        XCTAssertEqual(loaded?.primaryQuotas.count, 1)
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
