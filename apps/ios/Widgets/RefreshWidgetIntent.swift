import AppIntents
import WidgetKit

public struct RefreshWidgetIntent: AppIntent {
    public static var title: LocalizedStringResource = "刷新用量看板"
    public static var description = IntentDescription("从共享容器重新读取最新同步的统计与配额快照。")

    public init() {}

    public func perform() async throws -> some IntentResult {
        WidgetCenter.shared.reloadAllTimelines()
        return .result()
    }
}
