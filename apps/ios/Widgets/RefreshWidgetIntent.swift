import AppIntents
import WidgetKit

public struct RefreshWidgetIntent: AppIntent {
    public static let title: LocalizedStringResource = "widget.refresh.title"
    public static let description = IntentDescription("widget.refresh.desc")

    public init() {}

    public func perform() async throws -> some IntentResult {
        let synced = await WidgetRefreshBridge.shared.triggerIfRegistered()
        if !synced {
            WidgetCenter.shared.reloadAllTimelines()
        }
        return .result()
    }
}
