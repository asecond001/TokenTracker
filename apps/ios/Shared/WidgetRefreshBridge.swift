import Foundation

/// Thread-safe bridge letting `RefreshWidgetIntent` trigger an actual network sync when executed
/// inside the main app process, while still compiling cleanly inside the zero-Rust widget extension.
public final class WidgetRefreshBridge: @unchecked Sendable {
    public static let shared = WidgetRefreshBridge()

    private let lock = NSLock()
    private var refreshHandler: (@Sendable () async -> Bool)?

    private init() {}

    public func register(_ handler: @escaping @Sendable () async -> Bool) {
        lock.lock()
        refreshHandler = handler
        lock.unlock()
    }

    public func triggerIfRegistered() async -> Bool {
        lock.lock()
        let handler = refreshHandler
        lock.unlock()
        guard let handler else { return false }
        return await handler()
    }
}
