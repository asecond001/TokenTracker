import SwiftUI
import UIKit

private struct ZigZagReceiptPaperShape: Shape {
    let tooth: CGFloat

    func path(in rect: CGRect) -> Path {
        var path = Path()
        let w = rect.width
        let h = rect.height
        let step = tooth * 2

        path.move(to: CGPoint(x: 0, y: tooth))
        var x: CGFloat = 0
        while x < w {
            let nextPeak = min(w, x + tooth)
            let nextValley = min(w, x + step)
            path.addLine(to: CGPoint(x: nextPeak, y: 0))
            path.addLine(to: CGPoint(x: nextValley, y: tooth))
            x += step
        }

        path.addLine(to: CGPoint(x: w, y: h - tooth))
        x = w
        while x > 0 {
            let nextPeak = max(0, x - tooth)
            let nextValley = max(0, x - step)
            path.addLine(to: CGPoint(x: nextPeak, y: h))
            path.addLine(to: CGPoint(x: nextValley, y: h - tooth))
            x -= step
        }

        path.closeSubpath()
        return path
    }
}

private struct ReceiptPaperView: View {
    let model: ReceiptModel

    private let paperColor = Color(red: 0.984, green: 0.980, blue: 0.965) // #FBFAF6
    private let inkColor = Color(red: 0.122, green: 0.137, blue: 0.157) // #1F2328
    private let mutedColor = Color(red: 0.420, green: 0.439, blue: 0.475) // #6B7079
    private let ruleColor = Color(red: 0.725, green: 0.737, blue: 0.761) // #B9BCC2

    var body: some View {
        VStack(spacing: 4) {
            ForEach(Array(model.lines.enumerated()), id: \.offset) { _, line in
                lineView(line)
            }
        }
        .padding(.horizontal, 22)
        .padding(.vertical, 22)
        .frame(width: 360)
        .background(
            ZigZagReceiptPaperShape(tooth: 8)
                .fill(paperColor)
                .shadow(color: Color.black.opacity(0.14), radius: 10, x: 0, y: 4)
        )
    }

    @ViewBuilder
    private func lineView(_ line: ReceiptLine) -> some View {
        switch line {
        case .title(let text):
            Text(text.map { String($0) }.joined(separator: " "))
                .font(.system(size: 22, weight: .bold, design: .monospaced))
                .foregroundStyle(inkColor)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 2)

        case .subtitle(let text):
            Text(text)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(mutedColor)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.bottom, 2)

        case .divider:
            Path { path in
                path.move(to: CGPoint(x: 0, y: 0.5))
                path.addLine(to: CGPoint(x: 316, y: 0.5))
            }
            .stroke(ruleColor, style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            .frame(height: 6)

        case .header(let left, let mid, let right):
            HStack(spacing: 6) {
                Text(left)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(mid)
                    .frame(width: 68, alignment: .trailing)
                Text(right)
                    .frame(width: 78, alignment: .trailing)
            }
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(mutedColor)

        case .item(let left, let mid, let right):
            HStack(spacing: 6) {
                Text(left)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(mid)
                    .frame(width: 68, alignment: .trailing)
                Text(right)
                    .frame(width: 78, alignment: .trailing)
            }
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(inkColor)

        case .row(let left, let right, let strong):
            HStack {
                Text(left)
                    .font(.system(size: strong ? 13 : 12, weight: strong ? .bold : .regular, design: .monospaced))
                    .foregroundStyle(strong ? inkColor : mutedColor)
                Spacer()
                Text(right)
                    .font(.system(size: strong ? 13 : 12, weight: strong ? .bold : .medium, design: .monospaced))
                    .foregroundStyle(inkColor)
            }

        case .note(let text):
            Text(text)
                .font(.system(size: 11.5, design: .monospaced))
                .foregroundStyle(mutedColor)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.vertical, 2)

        case .barcode(let seed):
            VStack(spacing: 4) {
                Canvas { context, size in
                    let units = Array(seed.utf16)
                    guard !units.isEmpty else { return }
                    var x: CGFloat = 24
                    let end = size.width - 24
                    var i = 0
                    while x < end {
                        let code = Int(units[i % units.count]) + i * 7
                        let w = CGFloat((code % 3) + 1)
                        if i % 2 == 0 {
                            context.fill(
                                Path(CGRect(x: x, y: 2, width: w, height: 34)),
                                with: .color(inkColor)
                            )
                        }
                        x += w + CGFloat(((code >> 2) % 2) + 1)
                        i += 1
                    }
                }
                .frame(height: 38)

                Text(seed)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(mutedColor)
            }
            .padding(.top, 6)
        }
    }
}

public struct ShareReceiptSheet: View {
    @Environment(\.dismiss) private var dismiss
    public let report: MonthlyReportDTO
    public let currency: String
    public let usdToCnyRate: String
    public let periodLabel: String?

    @State private var exportedImageURL: URL?
    @State private var showingShareSheet = false

    public init(
        report: MonthlyReportDTO,
        currency: String,
        usdToCnyRate: String,
        periodLabel: String?
    ) {
        self.report = report
        self.currency = currency
        self.usdToCnyRate = usdToCnyRate
        self.periodLabel = periodLabel
    }

    public var body: some View {
        let model = ReceiptModelBuilder.buildReceiptModel(
            report: report,
            currency: currency,
            usdToCnyRate: usdToCnyRate,
            periodLabel: periodLabel
        )

        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    ReceiptPaperView(model: model)
                        .padding(.top, 16)
                        .padding(.horizontal, 12)

                    Button {
                        if let url = renderReceiptPNG(model: model) {
                            exportedImageURL = url
                            showingShareSheet = true
                        }
                    } label: {
                        Label(L10n.tr("receipt.action.share_png", default: "导出高清小票图片 (PNG)"), systemImage: "square.and.arrow.up")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white)
                            .frame(maxWidth: 360)
                            .padding(.vertical, 12)
                            .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                    .padding(.horizontal, 16)
                }
                .padding(.bottom, 24)
            }
            .background(SemanticColors.surfaceSubtle)
            .navigationTitle(L10n.tr("receipt.title", default: "用量小票"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.tr("common.close", default: "关闭")) {
                        dismiss()
                    }
                }
            }
            .sheet(isPresented: $showingShareSheet) {
                if let url = exportedImageURL {
                    ReceiptActivityShareView(url: url)
                }
            }
        }
    }

    @MainActor
    private func renderReceiptPNG(model: ReceiptModel) -> URL? {
        let content = ReceiptPaperView(model: model)
            .padding(20)
            .background(Color(red: 0.933, green: 0.941, blue: 0.957))
        let renderer = ImageRenderer(content: content)
        renderer.scale = 3.0
        guard let image = renderer.uiImage, let data = image.pngData() else {
            return nil
        }
        let targetURL = FileManager.default.temporaryDirectory.appendingPathComponent(model.fileName)
        try? data.write(to: targetURL, options: .atomic)
        return targetURL
    }
}

private struct ReceiptActivityShareView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}
