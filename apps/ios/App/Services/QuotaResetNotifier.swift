import Foundation
@preconcurrency import UserNotifications

public struct QuotaBaseline: Codable, Equatable, Sendable {
    public let usedPercent: Double
    public let resetsAtMs: Int64?
    public let updatedAtMs: Int64
    public let lastNotifiedResetToken: String?

    public init(
        usedPercent: Double,
        resetsAtMs: Int64?,
        updatedAtMs: Int64,
        lastNotifiedResetToken: String? = nil
    ) {
        self.usedPercent = usedPercent
        self.resetsAtMs = resetsAtMs
        self.updatedAtMs = updatedAtMs
        self.lastNotifiedResetToken = lastNotifiedResetToken
    }
}

public struct QuotaResetEvent: Equatable, Identifiable, Sendable {
    public let id: String
    public let deviceDisplayName: String
    public let providerId: String
    public let planName: String
    public let windowLabel: String
    public let previousPercent: Double
    public let currentPercent: Double

    public init(
        id: String,
        deviceDisplayName: String,
        providerId: String,
        planName: String,
        windowLabel: String,
        previousPercent: Double,
        currentPercent: Double
    ) {
        self.id = id
        self.deviceDisplayName = deviceDisplayName
        self.providerId = providerId
        self.planName = planName
        self.windowLabel = windowLabel
        self.previousPercent = previousPercent
        self.currentPercent = currentPercent
    }
}

public enum QuotaResetDetector {
    public static func detectResets(
        previous: [String: QuotaBaseline],
        current: [RemoteSubscriptionSnapshotDTO],
        nowMs: Int64
    ) -> (events: [QuotaResetEvent], baselines: [String: QuotaBaseline]) {
        var nextBaselines = previous
        var events: [QuotaResetEvent] = []
        let _ = nowMs

        for sub in current {
            let currentPct = sub.usedPercent.isFinite ? max(0.0, sub.usedPercent) : 0.0
            guard let prev = previous[sub.id] else {
                nextBaselines[sub.id] = QuotaBaseline(
                    usedPercent: currentPct,
                    resetsAtMs: sub.resetsAtMs,
                    updatedAtMs: sub.updatedAtMs,
                    lastNotifiedResetToken: nil
                )
                continue
            }

            let drop = prev.usedPercent - currentPct
            let significantDrop = (prev.usedPercent >= 15.0 && drop >= 15.0)
            let zeroedAfterUse = (prev.usedPercent >= 5.0 && currentPct < 1.0)
            let isChronological = sub.updatedAtMs >= prev.updatedAtMs
            let resetToken = "\(sub.resetsAtMs ?? 0):\(Int(currentPct.rounded()))"

            if isChronological && (significantDrop || zeroedAfterUse) && prev.lastNotifiedResetToken != resetToken {
                events.append(QuotaResetEvent(
                    id: sub.id,
                    deviceDisplayName: sub.deviceDisplayName,
                    providerId: sub.providerId,
                    planName: sub.planName,
                    windowLabel: sub.windowLabel,
                    previousPercent: prev.usedPercent,
                    currentPercent: currentPct
                ))
                nextBaselines[sub.id] = QuotaBaseline(
                    usedPercent: currentPct,
                    resetsAtMs: sub.resetsAtMs,
                    updatedAtMs: max(prev.updatedAtMs, sub.updatedAtMs),
                    lastNotifiedResetToken: resetToken
                )
            } else {
                let retainedToken = currentPct >= 10.0 ? nil : prev.lastNotifiedResetToken
                nextBaselines[sub.id] = QuotaBaseline(
                    usedPercent: isChronological ? currentPct : prev.usedPercent,
                    resetsAtMs: isChronological ? sub.resetsAtMs : prev.resetsAtMs,
                    updatedAtMs: max(prev.updatedAtMs, sub.updatedAtMs),
                    lastNotifiedResetToken: retainedToken
                )
            }
        }

        return (events, nextBaselines)
    }
}

@MainActor
public final class QuotaResetNotifier: NSObject, UNUserNotificationCenterDelegate {
    public static let shared = QuotaResetNotifier()

    private static let baselinesKey = "tokenvane.quota_reset_baselines_v1"
    private static let enabledKey = "tokenvane.quota_reset_notifications_enabled"

    private let defaults: UserDefaults
    private let customDeliver: (@Sendable ([QuotaResetEvent]) async -> Void)?
    private var baselines: [String: QuotaBaseline]
    private var didRequestPermission = false

    public var notificationsEnabled: Bool {
        get {
            if defaults.object(forKey: Self.enabledKey) == nil {
                return true
            }
            return defaults.bool(forKey: Self.enabledKey)
        }
        set {
            defaults.set(newValue, forKey: Self.enabledKey)
        }
    }

    public init(
        userDefaults: UserDefaults = .standard,
        installCenterDelegate: Bool = true,
        customDeliver: (@Sendable ([QuotaResetEvent]) async -> Void)? = nil
    ) {
        self.defaults = userDefaults
        self.customDeliver = customDeliver
        if let data = userDefaults.data(forKey: Self.baselinesKey),
           let decoded = try? JSONDecoder().decode([String: QuotaBaseline].self, from: data) {
            self.baselines = decoded
        } else {
            self.baselines = [:]
        }
        super.init()
        if installCenterDelegate && !Self.isRunningUnderTests {
            UNUserNotificationCenter.current().delegate = self
        }
    }

    private static var isRunningUnderTests: Bool {
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
            || ProcessInfo.processInfo.arguments.contains("--uitest-seed-demo")
            || ProcessInfo.processInfo.arguments.contains("--uitest-stage-pairing")
    }

    @discardableResult
    public func processSubscriptions(
        _ subscriptions: [RemoteSubscriptionSnapshotDTO],
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
    ) async -> [QuotaResetEvent] {
        let outcome = QuotaResetDetector.detectResets(
            previous: baselines,
            current: subscriptions,
            nowMs: nowMs
        )
        baselines = outcome.baselines
        persistBaselines()

        guard notificationsEnabled, !outcome.events.isEmpty else {
            return outcome.events
        }

        if let customDeliver {
            await customDeliver(outcome.events)
        } else if !Self.isRunningUnderTests {
            await deliverLocalNotifications(outcome.events)
        }
        return outcome.events
    }

    public func clearBaselines() {
        baselines = [:]
        defaults.removeObject(forKey: Self.baselinesKey)
    }

    private func persistBaselines() {
        if let encoded = try? JSONEncoder().encode(baselines) {
            defaults.set(encoded, forKey: Self.baselinesKey)
        }
    }

    private func deliverLocalNotifications(_ events: [QuotaResetEvent]) async {
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined:
            if !didRequestPermission {
                didRequestPermission = true
                let granted = (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
                guard granted else { return }
            } else {
                return
            }
        case .authorized, .provisional, .ephemeral:
            break
        case .denied:
            return
        @unknown default:
            return
        }

        for event in events {
            let content = UNMutableNotificationContent()
            content.title = L10n.trFormat("notification.quota_reset.title", default: "%@ 配额已重置", event.planName)
            content.body = L10n.trFormat(
                "notification.quota_reset.body",
                default: "%@ · %@ %lld%% → %lld%%",
                event.deviceDisplayName,
                event.windowLabel,
                Int64(event.previousPercent.rounded()),
                Int64(event.currentPercent.rounded())
            )
            content.sound = .default
            let identifier = "quota-reset-\(event.id)-\(Int(Date().timeIntervalSince1970))"
            let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
            try? await center.add(request)
        }
    }

    nonisolated public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }
}
