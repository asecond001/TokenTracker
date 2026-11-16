import ActivityKit
import Foundation

@MainActor
public final class LiveActivityManager {
    public static let shared = LiveActivityManager()

    private init() {}

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
            return false
        }

        let attributes = CodingSessionAttributes(
            deviceId: subscription.deviceId,
            deviceName: subscription.deviceDisplayName,
            providerId: subscription.providerId,
            providerName: TokenTrackerFormatters.providerDisplayName(subscription.providerId),
            planId: subscription.planId,
            planName: subscription.planName,
            windowId: subscription.windowId,
            windowLabel: subscription.windowLabel
        )
        let state = CodingSessionAttributes.ContentState(
            usedPercent: subscription.usedPercent,
            resetsAtEpochSeconds: subscription.resetsAtMs.map { TimeInterval($0) / 1000.0 },
            todayCostFormatted: todayCostFormatted,
            latestTps: latestTps,
            updatedAtEpochSeconds: Date().timeIntervalSince1970
        )
        let content = ActivityContent(state: state, staleDate: Date().addingTimeInterval(3600))
        _ = try Activity.request(attributes: attributes, content: content, pushType: nil)
        return true
    }

    public func refreshActiveActivities(
        subscriptions: [RemoteSubscriptionSnapshotDTO],
        snapshot: WidgetDashboardSnapshot?
    ) async {
        let costFormatted: String
        if let snapshot {
            costFormatted = TokenTrackerFormatters.formatCurrency(
                snapshot.todayCostMicroUsd,
                currency: snapshot.currency,
                rateString: snapshot.usdToCnyRate
            )
        } else {
            costFormatted = "$0.00"
        }

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
                updatedAtEpochSeconds: Date().timeIntervalSince1970
            )
            let content = ActivityContent(state: updatedState, staleDate: Date().addingTimeInterval(3600))
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
