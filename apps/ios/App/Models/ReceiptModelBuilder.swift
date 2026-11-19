import Foundation

public enum ReceiptLine: Equatable, Sendable {
    case title(text: String)
    case subtitle(text: String)
    case divider
    case header(left: String, mid: String, right: String)
    case item(left: String, mid: String, right: String)
    case row(left: String, right: String, strong: Bool = false)
    case note(text: String)
    case barcode(seed: String)
}

public struct ReceiptModel: Equatable, Sendable {
    public let lines: [ReceiptLine]
    public let fileName: String

    public init(lines: [ReceiptLine], fileName: String) {
        self.lines = lines
        self.fileName = fileName
    }
}

public enum ReceiptModelBuilder {
    private static let maxItems = 8
    private static let latteUsd = 4.5
    private static let latteCny = 30.0
    private static let posixLocale = Locale(identifier: "en_US_POSIX")

    private static func pad2(_ value: Int) -> String {
        String(format: "%02d", locale: posixLocale, value)
    }

    private static func truncate(_ text: String, maxLength: Int) -> String {
        guard text.count > maxLength, maxLength > 1 else { return text }
        return String(text.prefix(maxLength - 1)) + "…"
    }

    public static func receiptNo(report: MonthlyReportDTO) -> String {
        var h: UInt32 = 2_166_136_261
        let seed = "\(report.year)-\(report.month)-\(report.monthTotalTokens)-\(report.monthTotalRequests)"
        for codeUnit in seed.utf16 {
            h ^= UInt32(codeUnit)
            h = h &* 16_777_619
        }
        let hex = String(format: "%08X", locale: posixLocale, h)
        let suffix = String(hex.prefix(6))
        return "TV\(report.year)\(pad2(Int(report.month)))-\(suffix)"
    }

    private static func dayEpochIndex(ymd: String) -> Int? {
        let parts = ymd.split(separator: "-")
        guard parts.count == 3,
              let y = Int(parts[0]),
              let m = Int(parts[1]),
              let d = Int(parts[2]) else {
            return nil
        }
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(secondsFromGMT: 0) ?? .current
        guard let date = cal.date(from: DateComponents(year: y, month: m, day: d)) else {
            return nil
        }
        return Int((date.timeIntervalSince1970 / 86_400.0).rounded())
    }

    public static func longestActiveStreak(report: MonthlyReportDTO) -> Int {
        var best = 0
        var run = 0
        var prevDay: Int?
        let sortedDays = report.dailyLedger.sorted { $0.date < $1.date }
        for day in sortedDays {
            guard day.totalTokens > 0, let dayIndex = dayEpochIndex(ymd: day.date) else {
                run = 0
                prevDay = nil
                continue
            }
            if let prev = prevDay, dayIndex - prev == 1 {
                run += 1
            } else {
                run = 1
            }
            prevDay = dayIndex
            best = max(best, run)
        }
        return best
    }

    public static func buildReceiptModel(
        report: MonthlyReportDTO,
        currency: String,
        usdToCnyRate: String,
        now: Date = Date(),
        periodLabel: String? = nil
    ) -> ReceiptModel {
        func cost(_ micro: Int64) -> String {
            TokenVaneFormatters.formatCurrency(micro, currency: currency, rateString: usdToCnyRate)
        }

        let trimmedPeriod = periodLabel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let period = trimmedPeriod.isEmpty ? "\(report.year)-\(pad2(Int(report.month)))" : trimmedPeriod
        let invalidChars = CharacterSet(charactersIn: "\\/:*?\"<>|")
        var fileStem = period
            .components(separatedBy: invalidChars)
            .joined()
        fileStem = fileStem.replacingOccurrences(
            of: #"\s*~\s*"#,
            with: "_",
            options: .regularExpression
        )
        fileStem = fileStem.replacingOccurrences(
            of: #"\s+"#,
            with: "-",
            options: .regularExpression
        )

        let comps = Calendar.current.dateComponents([.year, .month, .day, .hour, .minute], from: now)
        let printedAt = "\(comps.year ?? 2026)-\(pad2(comps.month ?? 1))-\(pad2(comps.day ?? 1)) \(pad2(comps.hour ?? 0)):\(pad2(comps.minute ?? 0))"
        let serial = receiptNo(report: report)

        var lines: [ReceiptLine] = [
            .title(text: "TOKENVANE"),
            .subtitle(text: L10n.trFormat("receipt.subtitle", default: "AI 用量小票 · %@", period)),
            .divider,
            .row(left: L10n.tr("receipt.order_no", default: "单号"), right: serial),
            .row(left: L10n.tr("receipt.printed_at", default: "打印"), right: printedAt),
            .divider,
            .header(
                left: L10n.tr("receipt.col.item", default: "品名"),
                mid: "Token",
                right: L10n.tr("receipt.col.amount", default: "金额")
            )
        ]

        let models = report.modelShares
            .filter { $0.tokens > 0 || $0.costMicroUsd > 0 }
            .sorted { lhs, rhs in
                if lhs.costMicroUsd != rhs.costMicroUsd {
                    return lhs.costMicroUsd > rhs.costMicroUsd
                }
                return lhs.tokens > rhs.tokens
            }

        let shown = models.prefix(maxItems)
        for m in shown {
            let name = m.label.isEmpty ? m.key : m.label
            lines.append(.item(
                left: truncate(name, maxLength: 22),
                mid: TokenVaneFormatters.formatCompactTokens(m.tokens),
                right: cost(m.costMicroUsd)
            ))
        }

        let rest = models.dropFirst(maxItems)
        if !rest.isEmpty {
            let restTokens = rest.reduce(Int64(0)) { $0 + $1.tokens }
            let restCost = rest.reduce(Int64(0)) { $0 + $1.costMicroUsd }
            lines.append(.item(
                left: L10n.trFormat("receipt.other_models", default: "其他 %lld 个模型", Int64(rest.count)),
                mid: TokenVaneFormatters.formatCompactTokens(restTokens),
                right: cost(restCost)
            ))
        }

        if models.isEmpty {
            lines.append(.note(text: L10n.tr("receipt.empty", default: "等待第一笔消费")))
        }

        lines.append(.divider)
        lines.append(.row(
            left: L10n.tr("receipt.requests", default: "请求次数"),
            right: TokenVaneFormatters.formatExactInt(report.monthTotalRequests)
        ))
        lines.append(.row(
            left: L10n.tr("receipt.total_tokens", default: "Token 合计"),
            right: TokenVaneFormatters.formatCompactTokens(report.monthTotalTokens)
        ))
        lines.append(.row(
            left: L10n.tr("receipt.cache_saved", default: "缓存立减"),
            right: "-\(cost(report.monthCacheSavedMicroUsd))"
        ))
        lines.append(.divider)
        lines.append(.row(
            left: L10n.tr("receipt.api_total", default: "API 等效合计"),
            right: cost(report.apiEquivalentCostMicroUsd),
            strong: true
        ))

        let subscriptionFee = report.subscriptionPeriodFeeMicroUsd ?? report.subscriptionMonthlyFeeMicroUsd
        if subscriptionFee > 0 {
            lines.append(.row(
                left: L10n.tr("receipt.subscription_fee", default: "订阅实付"),
                right: cost(subscriptionFee)
            ))
            if let roi = report.roiMultiplier, roi.isFinite {
                lines.append(.row(
                    left: L10n.tr("receipt.roi", default: "回本倍数"),
                    right: String(format: "%.1f×", locale: posixLocale, roi),
                    strong: true
                ))
            }
        }

        let busiest = report.dailyLedger.max { $0.totalTokens < $1.totalTokens }
        let outputTokens = report.dailyLedger.reduce(Int64(0)) { $0 + $1.outputTokens }
        let streak = longestActiveStreak(report: report)
        let spend = TokenVaneFormatters.convertMicroUsd(
            report.apiEquivalentCostMicroUsd,
            currency: currency,
            rateString: usdToCnyRate
        )
        let lattePrice = currency.uppercased() == "CNY" ? latteCny : latteUsd
        let lattes = Int(floor(spend / lattePrice))

        lines.append(.divider)
        if let busiest, busiest.totalTokens > 0 {
            let shortDate = busiest.date.count > 5 ? String(busiest.date.dropFirst(5)) : busiest.date
            lines.append(.row(
                left: L10n.tr("receipt.busiest_day", default: "最忙一天"),
                right: "\(shortDate) · \(TokenVaneFormatters.formatCompactTokens(busiest.totalTokens))"
            ))
        }
        if streak > 0 {
            lines.append(.row(
                left: L10n.tr("receipt.streak", default: "连续活跃"),
                right: L10n.trFormat("receipt.streak_days", default: "%lld 天", Int64(streak))
            ))
        }
        if outputTokens > 0 {
            // Rough rule of thumb: one output token ≈ 1.5 Chinese characters.
            let wan = (Double(outputTokens) * 1.5) / 10_000.0
            let formattedWan = wan >= 100.0
                ? TokenVaneFormatters.formatExactInt(Int64(wan.rounded()))
                : String(format: "%.1f", locale: posixLocale, wan)
            lines.append(.row(
                left: L10n.tr("receipt.ai_writing", default: "AI 代笔"),
                right: L10n.trFormat("receipt.ai_writing_wan", default: "≈ %@ 万字", formattedWan)
            ))
        }
        if lattes > 0 {
            lines.append(.row(
                left: L10n.tr("receipt.lattes", default: "折合拿铁"),
                right: L10n.trFormat("receipt.lattes_cups", default: "≈ %@ 杯", TokenVaneFormatters.formatExactInt(Int64(lattes)))
            ))
        }

        lines.append(.barcode(seed: serial))
        lines.append(.note(text: L10n.tr("receipt.thanks", default: "谢谢惠顾 · 欢迎再来")))

        return ReceiptModel(lines: lines, fileName: "tokenvane-receipt-\(fileStem).png")
    }

    /// 5-level (0...4) quartile intensity mapping matching PC `computeHeatmapIntensityLevels`.
    public static func computeHeatmapIntensityLevels(_ cells: [HeatmapDayCellDTO]) -> [String: Int] {
        var levels: [String: Int] = [:]
        levels.reserveCapacity(cells.count)
        let positiveTokens = cells
            .filter { $0.totalTokens > 0 }
            .map(\.totalTokens)
            .sorted()

        func pickQuantile(_ p: Double) -> Int64 {
            guard !positiveTokens.isEmpty else { return 0 }
            let idx = min(
                positiveTokens.count - 1,
                max(0, Int(floor(Double(positiveTokens.count - 1) * p)))
            )
            return positiveTokens[idx]
        }

        let q25 = pickQuantile(0.25)
        let q50 = pickQuantile(0.50)
        let q75 = pickQuantile(0.75)
        let uniformPositive = !positiveTokens.isEmpty && q75 <= q25

        for c in cells {
            if c.totalTokens <= 0 {
                levels[c.date] = c.requests > 0 ? 1 : 0
                continue
            }
            if uniformPositive {
                levels[c.date] = 3
                continue
            }
            if c.totalTokens <= q25 {
                levels[c.date] = 1
            } else if c.totalTokens <= q50 {
                levels[c.date] = 2
            } else if c.totalTokens <= q75 {
                levels[c.date] = 3
            } else {
                levels[c.date] = 4
            }
        }
        return levels
    }
}
