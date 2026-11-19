import SwiftUI

public enum SemanticColors {
    public static let surfacePrimary = Color(uiColor: UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(red: 0.043, green: 0.047, blue: 0.055, alpha: 1.0) // #0B0C0E
            : UIColor(red: 1.000, green: 1.000, blue: 1.000, alpha: 1.0) // #FFFFFF
    })

    public static let surfaceSubtle = Color(uiColor: UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(red: 0.075, green: 0.082, blue: 0.098, alpha: 1.0) // #131519
            : UIColor(red: 0.969, green: 0.973, blue: 0.980, alpha: 1.0) // #F7F8FA
    })

    public static let hairline = Color.primary.opacity(0.09)
    public static let accentIndigo = Color(red: 0.369, green: 0.400, blue: 0.945) // #5E66F1
    public static let statusLive = Color(red: 0.063, green: 0.725, blue: 0.506)
    public static let statusWarning = Color(red: 0.961, green: 0.620, blue: 0.043)
    public static let statusDanger = Color(red: 0.937, green: 0.267, blue: 0.267)
}

public struct HairlineDivider: View {
    @Environment(\.displayScale) private var displayScale
    private let insetLeading: CGFloat

    public init(insetLeading: CGFloat = 0) {
        self.insetLeading = insetLeading
    }

    public var body: some View {
        Rectangle()
            .fill(SemanticColors.hairline)
            .frame(height: 1.0 / max(1.0, displayScale))
            .padding(.leading, insetLeading)
    }
}

public struct TabularMetricsModifier: ViewModifier {
    public func body(content: Content) -> some View {
        content.monospacedDigit()
    }
}

public extension View {
    func tabularMetrics() -> some View {
        modifier(TabularMetricsModifier())
    }
}

public struct ShareBarSegment: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let sharePercent: Double
    public let color: Color

    public init(id: String, label: String, sharePercent: Double, color: Color) {
        self.id = id
        self.label = label
        self.sharePercent = max(0, sharePercent)
        self.color = color
    }
}

public struct InlineShareBar: View {
    public let segments: [ShareBarSegment]
    public let height: CGFloat

    public init(segments: [ShareBarSegment], height: CGFloat = 6) {
        self.segments = segments
        self.height = height
    }

    private var totalWeight: Double {
        let sum = segments.reduce(0.0) { $0 + $1.sharePercent }
        return sum > 0 ? sum : 1.0
    }

    public var body: some View {
        GeometryReader { proxy in
            if segments.isEmpty {
                Capsule()
                    .fill(SemanticColors.hairline)
                    .frame(width: proxy.size.width, height: height)
            } else {
                HStack(spacing: 2) {
                    ForEach(segments) { segment in
                        let ratio = segment.sharePercent / totalWeight
                        Rectangle()
                            .fill(segment.color)
                            .frame(width: max(3, (proxy.size.width - CGFloat(max(0, segments.count - 1)) * 2) * ratio))
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 2, style: .continuous))
            }
        }
        .frame(height: height)
    }
}

public struct LinearProgressStrip: View {
    public let progressPercent: Double
    public let tint: Color

    public init(progressPercent: Double, tint: Color) {
        self.progressPercent = progressPercent
        self.tint = tint
    }

    public var body: some View {
        GeometryReader { proxy in
            let fraction = min(1.0, max(0.0, progressPercent / 100.0))
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: 2, style: .continuous)
                    .fill(SemanticColors.hairline)
                    .frame(height: 5)
                RoundedRectangle(cornerRadius: 2, style: .continuous)
                    .fill(tint)
                    .frame(width: max(fraction > 0 ? 4 : 0, proxy.size.width * fraction), height: 5)
            }
        }
        .frame(height: 5)
    }
}

public struct DeltaBadge: View {
    public let percent: Double?
    public let prefix: String

    public init(percent: Double?, prefix: String = L10n.tr("delta.vs_yesterday", default: "较昨日")) {
        self.percent = percent
        self.prefix = prefix
    }

    public var body: some View {
        if let percent, percent.isFinite {
            let positive = percent >= 0
            let sign = positive ? "+" : ""
            let formatted = String(format: "%@%.1f%%", locale: Locale(identifier: "en_US_POSIX"), sign, percent)
            Text("\(prefix) \(formatted)")
                .font(.caption2.weight(.medium))
                .tabularMetrics()
                .foregroundStyle(positive ? SemanticColors.statusLive : SemanticColors.statusDanger)
                .lineLimit(1)
        } else {
            Text("\(prefix) —")
                .font(.caption2)
                .tabularMetrics()
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

public struct BackdropPresetOption: Equatable, Identifiable, Sendable {
    public let id: String
    public let title: String
    public let subtitle: String
    public let darkStops: [Color]
    public let lightStops: [Color]

    public static let allPresets: [BackdropPresetOption] = [
        BackdropPresetOption(
            id: "aurora_slate",
            title: L10n.tr("backdrop.aurora_slate.title", default: "极光石板"),
            subtitle: L10n.tr("backdrop.aurora_slate.subtitle", default: "靛蓝 / 翠绿冷调微光"),
            darkStops: [
                Color(red: 0.388, green: 0.400, blue: 0.945, opacity: 0.85),
                Color(red: 0.063, green: 0.725, blue: 0.506, opacity: 0.62),
                Color(red: 0.220, green: 0.741, blue: 0.973, opacity: 0.44)
            ],
            lightStops: [
                Color(red: 0.310, green: 0.275, blue: 0.898, opacity: 0.42),
                Color(red: 0.020, green: 0.588, blue: 0.412, opacity: 0.34),
                Color(red: 0.008, green: 0.518, blue: 0.780, opacity: 0.28)
            ]
        ),
        BackdropPresetOption(
            id: "obsidian_mist",
            title: L10n.tr("backdrop.obsidian_mist.title", default: "黑曜薄雾"),
            subtitle: L10n.tr("backdrop.obsidian_mist.subtitle", default: "灰紫 / 岩灰低饱和雾光"),
            darkStops: [
                Color(red: 0.545, green: 0.361, blue: 0.965, opacity: 0.68),
                Color(red: 0.392, green: 0.455, blue: 0.545, opacity: 0.78),
                Color(red: 0.278, green: 0.333, blue: 0.412, opacity: 0.62)
            ],
            lightStops: [
                Color(red: 0.427, green: 0.157, blue: 0.851, opacity: 0.32),
                Color(red: 0.392, green: 0.455, blue: 0.545, opacity: 0.42),
                Color(red: 0.580, green: 0.639, blue: 0.722, opacity: 0.46)
            ]
        ),
        BackdropPresetOption(
            id: "nordic_frost",
            title: L10n.tr("backdrop.nordic_frost.title", default: "北欧霜蓝"),
            subtitle: L10n.tr("backdrop.nordic_frost.subtitle", default: "冰青 / 钴蓝清冽视域"),
            darkStops: [
                Color(red: 0.220, green: 0.741, blue: 0.973, opacity: 0.76),
                Color(red: 0.231, green: 0.510, blue: 0.965, opacity: 0.68),
                Color(red: 0.580, green: 0.639, blue: 0.722, opacity: 0.40)
            ],
            lightStops: [
                Color(red: 0.008, green: 0.518, blue: 0.780, opacity: 0.38),
                Color(red: 0.145, green: 0.388, blue: 0.922, opacity: 0.32),
                Color(red: 0.055, green: 0.647, blue: 0.914, opacity: 0.24)
            ]
        ),
        BackdropPresetOption(
            id: "amber_dusk",
            title: L10n.tr("backdrop.amber_dusk.title", default: "琥珀暮光"),
            subtitle: L10n.tr("backdrop.amber_dusk.subtitle", default: "暖金 / 霞红克制氛围"),
            darkStops: [
                Color(red: 0.961, green: 0.620, blue: 0.043, opacity: 0.72),
                Color(red: 0.957, green: 0.247, blue: 0.369, opacity: 0.52),
                Color(red: 0.388, green: 0.400, blue: 0.945, opacity: 0.42)
            ],
            lightStops: [
                Color(red: 0.851, green: 0.467, blue: 0.024, opacity: 0.38),
                Color(red: 0.882, green: 0.114, blue: 0.282, opacity: 0.26),
                Color(red: 0.310, green: 0.275, blue: 0.898, opacity: 0.22)
            ]
        )
    ]

    public static func resolve(_ id: String?) -> BackdropPresetOption {
        allPresets.first(where: { $0.id == id }) ?? allPresets[0]
    }
}

public struct WorkspaceBackdropView: View {
    @Environment(\.colorScheme) private var colorScheme
    public let mode: String
    public let presetId: String
    public let opacity: Double

    public init(mode: String, presetId: String, opacity: Double) {
        self.mode = mode
        self.presetId = presetId
        self.opacity = opacity
    }

    public var body: some View {
        let clampedOpacity = min(0.6, max(0.0, opacity.isFinite ? opacity : 0.18))
        let preset = BackdropPresetOption.resolve(presetId)
        let stops = colorScheme == .dark ? preset.darkStops : preset.lightStops

        ZStack {
            SemanticColors.surfacePrimary
            if mode == "gradient" && clampedOpacity > 0.005 {
                GeometryReader { geo in
                    ZStack {
                        RadialGradient(
                            colors: [stops[0], stops[0].opacity(0)],
                            center: UnitPoint(x: 0.14, y: 0.08),
                            startRadius: 0,
                            endRadius: max(geo.size.width, geo.size.height) * 0.65
                        )
                        RadialGradient(
                            colors: [stops[1], stops[1].opacity(0)],
                            center: UnitPoint(x: 0.86, y: 0.14),
                            startRadius: 0,
                            endRadius: max(geo.size.width, geo.size.height) * 0.58
                        )
                        RadialGradient(
                            colors: [stops[2], stops[2].opacity(0)],
                            center: UnitPoint(x: 0.52, y: 0.94),
                            startRadius: 0,
                            endRadius: max(geo.size.width, geo.size.height) * 0.68
                        )
                    }
                    .opacity(clampedOpacity)
                }
            }
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
    }
}
