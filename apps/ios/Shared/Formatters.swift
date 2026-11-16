import Foundation
import SwiftUI

public enum TokenTrackerFormatters {
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
        if abs(amount) >= 100.0 {
            return String(format: "%@%.1f", symbol, amount)
        }
        if abs(amount) >= 1.0 || abs(amount) < 0.0001 {
            return String(format: "%@%.2f", symbol, amount)
        }
        return String(format: "%@%.3f", symbol, amount)
    }

    public static func formatCompactTokens(_ tokens: Int64) -> String {
        let absVal = Double(abs(tokens))
        let sign = tokens < 0 ? "-" : ""
        if absVal >= 1_000_000_000 {
            return String(format: "%@%.2fB", sign, absVal / 1_000_000_000.0)
        }
        if absVal >= 1_000_000 {
            return String(format: "%@%.2fM", sign, absVal / 1_000_000.0)
        }
        if absVal >= 1_000 {
            return String(format: "%@%.1fK", sign, absVal / 1_000.0)
        }
        return "\(tokens)"
    }

    public static func formatExactInt(_ value: Int64) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.groupingSeparator = ","
        return formatter.string(from: NSNumber(value: value)) ?? "\(value)"
    }

    public static func formatPercent(_ value: Double, decimals: Int = 1) -> String {
        let clamped = value.isFinite ? max(0.0, min(999.9, value)) : 0.0
        return String(format: "%.\(decimals)f%%", clamped)
    }

    public static func formatTps(_ tps: Double?) -> String {
        guard let tps, tps.isFinite, tps > 0 else { return "—" }
        return String(format: "%.1f tok/s", tps)
    }

    public static func formatDurationMs(_ ms: Double) -> String {
        guard ms.isFinite, ms > 0 else { return "—" }
        if ms >= 1000.0 {
            return String(format: "%.2fs", ms / 1000.0)
        }
        return String(format: "%.0fms", ms)
    }

    public static func formatCountdown(untilMs: Int64?, now: Date = Date()) -> String {
        guard let untilMs else { return "—" }
        let remainingSec = Int(max(0, Double(untilMs) / 1000.0 - now.timeIntervalSince1970))
        if remainingSec <= 0 {
            return "即将重置"
        }
        let days = remainingSec / 86_400
        let hours = (remainingSec % 86_400) / 3_600
        let minutes = (remainingSec % 3_600) / 60
        let seconds = remainingSec % 60
        if days > 0 {
            return "\(days)d \(hours)h"
        }
        if hours > 0 {
            return String(format: "%dh %02dm", hours, minutes)
        }
        return String(format: "%02d:%02d", minutes, seconds)
    }

    public static func formatRelativeSync(atMs: Int64?, now: Date = Date()) -> String {
        guard let atMs, atMs > 0 else { return "未同步" }
        let deltaSec = max(0, Int(now.timeIntervalSince1970 - Double(atMs) / 1000.0))
        if deltaSec < 15 { return "刚刚" }
        if deltaSec < 60 { return "\(deltaSec) 秒前" }
        if deltaSec < 3600 { return "\(deltaSec / 60) 分钟前" }
        if deltaSec < 86400 { return "\(deltaSec / 3600) 小时前" }
        return "\(deltaSec / 86400) 天前"
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
