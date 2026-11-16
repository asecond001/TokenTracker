import ActivityKit
import Foundation

public struct CodingSessionAttributes: ActivityAttributes, Sendable {
    public struct ContentState: Codable, Hashable, Sendable {
        public var usedPercent: Double
        public var resetsAtEpochSeconds: TimeInterval?
        public var todayCostFormatted: String
        public var latestTps: Double?
        public var updatedAtEpochSeconds: TimeInterval

        public var resetsAtDate: Date? {
            guard let resetsAtEpochSeconds else { return nil }
            return Date(timeIntervalSince1970: resetsAtEpochSeconds)
        }

        public init(
            usedPercent: Double,
            resetsAtEpochSeconds: TimeInterval?,
            todayCostFormatted: String,
            latestTps: Double?,
            updatedAtEpochSeconds: TimeInterval
        ) {
            self.usedPercent = usedPercent
            self.resetsAtEpochSeconds = resetsAtEpochSeconds
            self.todayCostFormatted = todayCostFormatted
            self.latestTps = latestTps
            self.updatedAtEpochSeconds = updatedAtEpochSeconds
        }
    }

    public let deviceId: String
    public let deviceName: String
    public let providerId: String
    public let providerName: String
    public let planId: String
    public let planName: String
    public let windowId: String
    public let windowLabel: String

    public init(
        deviceId: String,
        deviceName: String,
        providerId: String,
        providerName: String,
        planId: String,
        planName: String,
        windowId: String,
        windowLabel: String
    ) {
        self.deviceId = deviceId
        self.deviceName = deviceName
        self.providerId = providerId
        self.providerName = providerName
        self.planId = planId
        self.planName = planName
        self.windowId = windowId
        self.windowLabel = windowLabel
    }
}
