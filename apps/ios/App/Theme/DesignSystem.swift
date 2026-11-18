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
