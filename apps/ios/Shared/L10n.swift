import Foundation

public enum L10n {
    public static func tr(_ key: String, default fallback: String) -> String {
        Bundle.main.localizedString(forKey: key, value: fallback, table: "Localizable")
    }

    public static func trFormat(_ key: String, default fallback: String, _ args: CVarArg...) -> String {
        let format = tr(key, default: fallback)
        return String(format: format, locale: Locale.current, arguments: args)
    }
}
