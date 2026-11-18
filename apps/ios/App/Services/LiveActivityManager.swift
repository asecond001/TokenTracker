@preconcurrency import ActivityKit
import Foundation

public enum LiveActivityError: LocalizedError, Equatable, Sendable {
    case activitiesDisabled

    public var errorDescription: String? {
        switch self {
        case .activitiesDisabled:
            return L10n.tr("live_activity.disabled", default: "请在系统设置开启实时活动权限")
        }
    }
}

@MainActor
public final class LiveActivityManager {
    public static let shared = LiveActivityManager()

    private init() {}

    public static func computeStaleDate(resetsAtMs: Int64?, now: Date = Date()) -> Date {
        guard let resetsAtMs else {
            return now.addingTimeInterval(1800)
        }
        let resetDate = Date(timeIntervalSince1970: Double(resetsAtMs) / 1000.0)
        return resetDate > now ? resetDate : now.addingTimeInterval(60)
    }

    public func isPinned(deviceId: String, planId: String, windowId: String) -> Bool {
        activeActivity(deviceId: deviceId, planId: planId, windowId: windowId) != nil
    }

    public func toggleWatch(
        subscription: RemoteSubscriptionSnapshotDTO,
        todayCostFormatted: String,
        latestTps: Double?
    ) async throws -> Bool {
        if let existing = activeActivity(
            deviceId: subscription.deviceId,
            planId: subscription.planId,
            windowId: subscription.windowId
        ) {
            await existing.end(nil, dismissalPolicy: .immediate)
            return false
        }

        guard ActivityAuthorizationInfo().areActivitiesEnabled else {
            throw LiveActivityError.activitiesDisabled
        }

        let attributes = CodingSessionAttributes(
            deviceId: subscription.deviceId,
            deviceName: subscription.deviceDisplayName,
            providerId: subscription.providerId,
            providerName: TokenVaneFormatters.providerDisplayName(subscription.providerId),
            planId: subscription.planId,
            planName: subscription.planName,
            windowId: subscription.windowId,
            windowLabel: subscription.windowLabel
        )
        let now = Date()
        let state = CodingSessionAttributes.ContentState(
            usedPercent: subscription.usedPercent,
            resetsAtEpochSeconds: subscription.resetsAtMs.map { TimeInterval($0) / 1000.0 },
            todayCostFormatted: todayCostFormatted,
            latestTps: latestTps,
            updatedAtEpochSeconds: now.timeIntervalSince1970
        )
        let staleDate = Self.computeStaleDate(resetsAtMs: subscription.resetsAtMs, now: now)
        let content = ActivityContent(state: state, staleDate: staleDate)
        _ = try Activity.request(attributes: attributes, content: content, pushType: nil)
        return true
    }

    public func refreshActiveActivities(
        subscriptions: [RemoteSubscriptionSnapshotDTO],
        snapshot: WidgetDashboardSnapshot?
    ) async {
        let costFormatted: String
        if let snapshot {
            costFormatted = TokenVaneFormatters.formatCurrency(
                snapshot.todayCostMicroUsd,
                currency: snapshot.currency,
                rateString: snapshot.usdToCnyRate
            )
        } else {
            costFormatted = "$0.00"
        }

        let now = Date()
        for activity in Activity<CodingSessionAttributes>.activities {
            let match = subscriptions.first {
                $0.deviceId == activity.attributes.deviceId &&
                $0.planId == activity.attributes.planId &&
                $0.windowId == activity.attributes.windowId
            }
            guard let match else { continue }
            let updatedState = CodingSessionAttributes.ContentState(
                usedPercent: match.usedPercent,
                resetsAtEpochSeconds: match.resetsAtMs.map { TimeInterval($0) / 1000.0 },
                todayCostFormatted: costFormatted,
                latestTps: snapshot?.latestOutputTps,
                updatedAtEpochSeconds: now.timeIntervalSince1970
            )
            let staleDate = Self.computeStaleDate(resetsAtMs: match.resetsAtMs, now: now)
            let content = ActivityContent(state: updatedState, staleDate: staleDate)
            await activity.update(content)
        }
    }

    public func activeWatchKeys() -> Set<String> {
        Set(Activity<CodingSessionAttributes>.activities.map {
            "\($0.attributes.deviceId)::\($0.attributes.planId)::\($0.attributes.windowId)"
        })
    }

    private func activeActivity(
        deviceId: String,
        planId: String,
        windowId: String
    ) -> Activity<CodingSessionAttributes>? {
        Activity<CodingSessionAttributes>.activities.first {
            $0.attributes.deviceId == deviceId &&
            $0.attributes.planId == planId &&
            $0.attributes.windowId == windowId
        }
    }
}
