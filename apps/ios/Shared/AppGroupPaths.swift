import Foundation

public enum AppGroupPaths {
    public static let defaultGroupIdentifier = "group.com.asecond.tokentracker"
    public static let defaultRefreshTaskIdentifier = "com.asecond.tokentracker.sync-refresh"

    public static var appGroupIdentifier: String {
        if let configured = Bundle.main.object(forInfoDictionaryKey: "TTAppGroupIdentifier") as? String {
            let trimmed = configured.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty, !trimmed.contains("$(") {
                return trimmed
            }
        }
        if let bundleId = Bundle.main.bundleIdentifier {
            if bundleId.hasSuffix(".widgets") {
                let base = String(bundleId.dropLast(".widgets".count))
                return "group.\(base)"
            }
            if !bundleId.hasSuffix(".tests"), !bundleId.hasSuffix(".uitests") {
                return "group.\(bundleId)"
            }
        }
        return defaultGroupIdentifier
    }

    public static var syncRefreshTaskIdentifier: String {
        if let configured = Bundle.main.object(forInfoDictionaryKey: "TTSyncRefreshTaskIdentifier") as? String {
            let trimmed = configured.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty, !trimmed.contains("$(") {
                return trimmed
            }
        }
        if let bundleId = Bundle.main.bundleIdentifier,
           !bundleId.hasSuffix(".widgets"),
           !bundleId.hasSuffix(".tests"),
           !bundleId.hasSuffix(".uitests") {
            return "\(bundleId).sync-refresh"
        }
        return defaultRefreshTaskIdentifier
    }

    public static func containerURL(fileManager: FileManager = .default) -> URL {
        if let groupURL = fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier) {
            return groupURL
        }
        return fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fileManager.temporaryDirectory
    }

    public static func databaseDirectoryURL(fileManager: FileManager = .default) throws -> URL {
        let dir = containerURL(fileManager: fileManager).appendingPathComponent("Database", isDirectory: true)
        try ensureProtectedDirectory(dir, fileManager: fileManager)
        return dir
    }

    public static func databaseFileURL(fileManager: FileManager = .default) throws -> URL {
        try databaseDirectoryURL(fileManager: fileManager)
            .appendingPathComponent("tokentracker.sqlite", isDirectory: false)
    }

    public static func snapshotsDirectoryURL(fileManager: FileManager = .default) throws -> URL {
        let dir = containerURL(fileManager: fileManager).appendingPathComponent("Snapshots", isDirectory: true)
        try ensureProtectedDirectory(dir, fileManager: fileManager)
        return dir
    }

    public static func widgetSnapshotFileURL(fileManager: FileManager = .default) throws -> URL {
        try snapshotsDirectoryURL(fileManager: fileManager)
            .appendingPathComponent("widget-dashboard.json", isDirectory: false)
    }

    public static func ensureProtectedDirectory(_ url: URL, fileManager: FileManager = .default) throws {
        if !fileManager.fileExists(atPath: url.path) {
            try fileManager.createDirectory(
                at: url,
                withIntermediateDirectories: true,
                attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
            )
        } else {
            try? fileManager.setAttributes(
                [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                ofItemAtPath: url.path
            )
        }
    }

    public static func writeWidgetSnapshot(
        _ snapshot: WidgetDashboardSnapshot,
        to customURL: URL? = nil,
        fileManager: FileManager = .default
    ) throws {
        let targetURL: URL
        if let customURL {
            try ensureProtectedDirectory(customURL.deletingLastPathComponent(), fileManager: fileManager)
            targetURL = customURL
        } else {
            targetURL = try widgetSnapshotFileURL(fileManager: fileManager)
        }
        let data = try JSONEncoder().encode(snapshot)
        try data.write(to: targetURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    public static func loadWidgetSnapshot(
        from customURL: URL? = nil,
        fileManager: FileManager = .default
    ) -> WidgetDashboardSnapshot? {
        let sourceURL: URL
        if let customURL {
            sourceURL = customURL
        } else if let groupURL = fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier) {
            sourceURL = groupURL
                .appendingPathComponent("Snapshots", isDirectory: true)
                .appendingPathComponent("widget-dashboard.json", isDirectory: false)
        } else {
            return nil
        }
        guard fileManager.fileExists(atPath: sourceURL.path),
              let data = try? Data(contentsOf: sourceURL) else {
            return nil
        }
        return try? JSONDecoder().decode(WidgetDashboardSnapshot.self, from: data)
    }
}
