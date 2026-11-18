import Foundation
import SwiftUI

private final class FormatterCache: @unchecked Sendable {
    private let lock = NSLock()
    private let posixLocale = Locale(identifier: "en_US_POSIX")

    private lazy var intFormatter: NumberFormatter = {
        let f = NumberFormatter()
        f.locale = posixLocale
        f.numberStyle = .decimal
        f.usesGroupingSeparator = true
        f.groupingSeparator = ","
        f.groupingSize = 3
        f.maximumFractionDigits = 0
        return f
    }()

    private lazy var currency1Formatter: NumberFormatter = makeCurrencyFormatter(decimals: 1)
    private lazy var currency2Formatter: NumberFormatter = makeCurrencyFormatter(decimals: 2)
    private lazy var currency3Formatter: NumberFormatter = makeCurrencyFormatter(decimals: 3)

    private lazy var bucketDateFormatter: DateFormatter = makeDateFormatter("MM-dd")
    private lazy var bucketDateTimeFormatter: DateFormatter = makeDateFormatter("MM-dd HH:mm")
    private lazy var bucketTimeOnlyFormatter: DateFormatter = makeDateFormatter("HH:mm")
    private lazy var watermarkFormatter: DateFormatter = makeDateFormatter("yyyy-MM-dd HH:mm:ss")
    private lazy var dayKeyFormatter: DateFormatter = makeDateFormatter("yyyy-MM-dd")

    private func makeCurrencyFormatter(decimals: Int) -> NumberFormatter {
        let f = NumberFormatter()
        f.locale = posixLocale
        f.numberStyle = .decimal
        f.usesGroupingSeparator = true
        f.groupingSeparator = ","
        f.groupingSize = 3
        f.minimumFractionDigits = decimals
        f.maximumFractionDigits = decimals
        return f
    }

    private func makeDateFormatter(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.locale = posixLocale
        f.dateFormat = format
        return f
    }

    func formatInt(_ value: Int64) -> String {
        lock.lock()
        defer { lock.unlock() }
        return intFormatter.string(from: NSNumber(value: value)) ?? "\(value)"
    }

    func formatDecimal(_ value: Double, decimals: Int) -> String {
        lock.lock()
        defer { lock.unlock() }
        let formatter: NumberFormatter
        switch decimals {
        case 1: formatter = currency1Formatter
        case 3: formatter = currency3Formatter
        default: formatter = currency2Formatter
        }
        return formatter.string(from: NSNumber(value: value)) ?? String(format: "%.\(decimals)f", value)
    }

    func formatDate(_ date: Date, kind: DateKind, timeZone: TimeZone) -> String {
        lock.lock()
        defer { lock.unlock() }
        let formatter: DateFormatter
        switch kind {
        case .bucketDate: formatter = bucketDateFormatter
        case .bucketDateTime: formatter = bucketDateTimeFormatter
        case .bucketTimeOnly: formatter = bucketTimeOnlyFormatter
        case .watermark: formatter = watermarkFormatter
        case .dayKey: formatter = dayKeyFormatter
        }
        formatter.timeZone = timeZone
        return formatter.string(from: date)
    }

    enum DateKind {
        case bucketDate
        case bucketDateTime
        case bucketTimeOnly
        case watermark
        case dayKey
    }
}

public enum TokenVaneFormatters {
    private static let cache = FormatterCache()
    private static let posixLocale = Locale(identifier: "en_US_POSIX")
    private static let modelPalette: [Color] = [
        Color(red: 0.369, green: 0.400, blue: 0.945),
        Color(red: 0.063, green: 0.639, blue: 0.498),
        Color(red: 0.961, green: 0.620, blue: 0.043),
        Color(red: 0.851, green: 0.467, blue: 0.341),
        Color(red: 0.024, green: 0.714, blue: 0.831),
        Color(red: 0.925, green: 0.282, blue: 0.600)
    ]

    public static func validateExchangeRate(_ raw: String) -> Double? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Double(trimmed), parsed.isFinite, parsed >= 0.1, parsed <= 100.0 else {
            return nil
        }
        return parsed
    }

    public static func exchangeRate(from rateString: String) -> Double {
        let parsed = Double(rateString.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 7.25
        return parsed.isFinite && parsed > 0 ? parsed : 7.25
    }

    public static func convertMicroUsd(
        _ microUsd: Int64,
        currency: String,
        rateString: String
    ) -> Double {
        let usd = Double(microUsd) / 1_000_000.0
        if currency.uppercased() == "CNY" {
            return usd * exchangeRate(from: rateString)
        }
        return usd
    }

    public static func formatCurrency(
        _ microUsd: Int64,
        currency: String,
        rateString: String
    ) -> String {
        let amount = convertMicroUsd(microUsd, currency: currency, rateString: rateString)
        let symbol = currency.uppercased() == "CNY" ? "¥" : "$"
        let absAmount = abs(amount)
        let sign = (amount < 0 && absAmount >= 0.00005) ? "-" : ""
        let decimals: Int
        if absAmount >= 100.0 {
            decimals = 1
        } else if absAmount >= 1.0 || absAmount < 0.0001 {
            decimals = 2
        } else {
            decimals = 3
        }
        let numberPart = cache.formatDecimal(absAmount, decimals: decimals)
        return "\(sign)\(symbol)\(numberPart)"
    }

    public static func formatCompactTokens(_ tokens: Int64) -> String {
        let absVal = Double(abs(tokens))
        let sign = tokens < 0 ? "-" : ""
        if absVal >= 1_000_000_000 {
            return String(format: "%@%.2fB", locale: posixLocale, sign, absVal / 1_000_000_000.0)
        }
        if absVal >= 1_000_000 {
            return String(format: "%@%.2fM", locale: posixLocale, sign, absVal / 1_000_000.0)
        }
        if absVal >= 1_000 {
            return String(format: "%@%.1fK", locale: posixLocale, sign, absVal / 1_000.0)
        }
        return "\(tokens)"
    }

    public static func formatExactInt(_ value: Int64) -> String {
        cache.formatInt(value)
    }

    public static func formatPercent(_ value: Double, decimals: Int = 1) -> String {
        let clamped = value.isFinite ? max(0.0, min(999.9, value)) : 0.0
        return String(format: "%.\(decimals)f%%", locale: posixLocale, clamped)
    }

    public static func formatTps(_ tps: Double?) -> String {
        guard let tps, tps.isFinite, tps > 0 else { return "—" }
        return String(format: "%.1f tok/s", locale: posixLocale, tps)
    }

    public static func formatDurationMs(_ ms: Double) -> String {
        guard ms.isFinite, ms > 0 else { return "—" }
        if ms >= 1000.0 {
            return String(format: "%.2fs", locale: posixLocale, ms / 1000.0)
        }
        return String(format: "%.0fms", locale: posixLocale, ms)
    }

    public static func formatBucketTimestamp(
        _ ms: Int64,
        isDateOnly: Bool,
        timeZone: TimeZone = .current
    ) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000.0)
        return cache.formatDate(date, kind: isDateOnly ? .bucketDate : .bucketDateTime, timeZone: timeZone)
    }

    public static func formatChartAxisDate(
        _ date: Date,
        subDay: Bool,
        timeZone: TimeZone = .current
    ) -> String {
        cache.formatDate(date, kind: subDay ? .bucketTimeOnly : .bucketDate, timeZone: timeZone)
    }

    public static func formatWatermarkTimestamp(
        _ ms: Int64?,
        timeZone: TimeZone = .current
    ) -> String {
        guard let ms, ms > 0 else {
            return L10n.tr("sync.status.offline", default: "待同步")
        }
        let date = Date(timeIntervalSince1970: Double(ms) / 1000.0)
        return cache.formatDate(date, kind: .watermark, timeZone: timeZone)
    }

    public static func formatDayKey(
        _ date: Date,
        timeZone: TimeZone = .current
    ) -> String {
        cache.formatDate(date, kind: .dayKey, timeZone: timeZone)
    }

    public static func formatCountdown(untilMs: Int64?, now: Date = Date()) -> String {
        guard let untilMs else { return "—" }
        let remainingSec = Int(max(0, Double(untilMs) / 1000.0 - now.timeIntervalSince1970))
        if remainingSec <= 0 {
            return L10n.tr("quota.resetting_soon", default: "即将重置")
        }
        let days = remainingSec / 86_400
        let hours = (remainingSec % 86_400) / 3_600
        let minutes = (remainingSec % 3_600) / 60
        let seconds = remainingSec % 60
        if days > 0 {
            return "\(days)d \(hours)h"
        }
        if hours > 0 {
            return String(format: "%dh %02dm", locale: posixLocale, hours, minutes)
        }
        return String(format: "%02d:%02d", locale: posixLocale, minutes, seconds)
    }

    public static func formatRelativeSync(atMs: Int64?, now: Date = Date()) -> String {
        guard let atMs, atMs > 0 else {
            return L10n.tr("sync.status.offline", default: "待同步")
        }
        let deltaSec = max(0, Int(now.timeIntervalSince1970 - Double(atMs) / 1000.0))
        if deltaSec < 15 { return L10n.tr("sync.relative.just_now", default: "刚刚") }
        if deltaSec < 60 { return L10n.trFormat("sync.relative.seconds_ago", default: "%lld秒前", Int64(deltaSec)) }
        if deltaSec < 3600 { return L10n.trFormat("sync.relative.minutes_ago", default: "%lld分前", Int64(deltaSec / 60)) }
        if deltaSec < 86400 { return L10n.trFormat("sync.relative.hours_ago", default: "%lld时前", Int64(deltaSec / 3600)) }
        return L10n.trFormat("sync.relative.days_ago", default: "%lld天前", Int64(deltaSec / 86400))
    }

    public static func providerShortCode(_ providerId: String) -> String {
        switch providerId {
        case "claude_code": return "CC"
        case "codex": return "CX"
        case "antigravity": return "AG"
        case "kimi_code": return "KM"
        case "deepseek_harness": return "DS"
        case "qcode": return "QC"
        case "minimax_code": return "MM"
        case "workbuddy": return "WB"
        default: return String(providerId.prefix(2)).uppercased()
        }
    }

    public static func providerDisplayName(_ providerId: String) -> String {
        switch providerId {
        case "claude_code": return "Claude Code"
        case "codex": return "Codex"
        case "antigravity": return "Antigravity"
        case "kimi_code": return "Kimi Code"
        case "deepseek_harness": return "DeepSeek Harness"
        case "qcode": return "Qcode"
        case "minimax_code": return "MiniMax Code"
        case "workbuddy": return "Workbuddy"
        default: return providerId
        }
    }

    public static func providerColor(_ providerId: String) -> Color {
        switch providerId {
        case "claude_code": return Color(red: 0.851, green: 0.467, blue: 0.341)
        case "codex": return Color(red: 0.063, green: 0.639, blue: 0.498)
        case "antigravity": return Color(red: 0.388, green: 0.400, blue: 0.945)
        case "kimi_code": return Color(red: 0.024, green: 0.714, blue: 0.831)
        case "deepseek_harness": return Color(red: 0.231, green: 0.510, blue: 0.965)
        case "qcode": return Color(red: 0.961, green: 0.620, blue: 0.043)
        case "minimax_code": return Color(red: 0.925, green: 0.282, blue: 0.600)
        case "workbuddy": return Color(red: 0.545, green: 0.361, blue: 0.965)
        default: return Color(red: 0.541, green: 0.561, blue: 0.616)
        }
    }

    public static func tokenTypeColor(_ key: String) -> Color {
        switch key {
        case "input": return Color(red: 0.369, green: 0.400, blue: 0.945)
        case "output": return Color(red: 0.063, green: 0.639, blue: 0.498)
        case "cache_read": return Color(red: 0.024, green: 0.714, blue: 0.831)
        case "cache_write": return Color(red: 0.961, green: 0.620, blue: 0.043)
        case "reasoning": return Color(red: 0.851, green: 0.467, blue: 0.341)
        default: return Color(red: 0.541, green: 0.561, blue: 0.616)
        }
    }

    public static func seriesColor(dimensionKey: String, seriesKey: String, index: Int) -> Color {
        switch dimensionKey {
        case "provider":
            return providerColor(seriesKey)
        case "token_type":
            return tokenTypeColor(seriesKey)
        default:
            return modelPalette[abs(index) % modelPalette.count]
        }
    }

    public static func quotaSeverityColor(usedPercent: Double) -> Color {
        if usedPercent >= 95.0 {
            return Color(red: 0.937, green: 0.267, blue: 0.267)
        }
        if usedPercent >= 80.0 {
            return Color(red: 0.961, green: 0.620, blue: 0.043)
        }
        return Color(red: 0.369, green: 0.400, blue: 0.945)
    }
}

public struct QuotaCountdownText: View {
    public let resetsAtMs: Int64?
    public let referenceDate: Date

    public init(resetsAtMs: Int64?, referenceDate: Date = Date()) {
        self.resetsAtMs = resetsAtMs
        self.referenceDate = referenceDate
    }

    public var body: some View {
        if let resetsAtMs {
            let resetDate = Date(timeIntervalSince1970: Double(resetsAtMs) / 1000.0)
            let remaining = resetDate.timeIntervalSince(referenceDate)
            if remaining <= 0 {
                Text(L10n.tr("quota.resetting_soon", default: "即将重置"))
            } else if remaining >= 86_400 {
                Text(TokenVaneFormatters.formatCountdown(untilMs: resetsAtMs, now: referenceDate))
            } else {
                Text(timerInterval: referenceDate...resetDate, countsDown: true)
            }
        } else {
            Text("—")
        }
    }
}
