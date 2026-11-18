@preconcurrency import AVFoundation
import SwiftUI

public struct QRScannerSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var manualPayload = ""
    @State private var cameraError: String?

    public let onScanned: (String) -> Void

    public init(onScanned: @escaping (String) -> Void) {
        self.onScanned = onScanned
    }

    public var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 16) {
                #if targetEnvironment(simulator)
                simulatorPlaceholder
                #else
                if AVCaptureDevice.authorizationStatus(for: .video) == .denied {
                    cameraDeniedBanner
                } else {
                    CameraQRPreview { code in
                        onScanned(code)
                        dismiss()
                    } onFailure: { msg in
                        cameraError = msg
                    }
                    .frame(height: 260)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
                }
                #endif

                if let cameraError {
                    Text(cameraError)
                        .font(.caption)
                        .foregroundStyle(SemanticColors.statusDanger)
                }

                HairlineDivider()

                Text(L10n.tr("scanner.paste_prompt", default: "或粘贴配对链接 / JSON"))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)

                TextEditor(text: $manualPayload)
                    .font(.caption.monospaced())
                    .frame(height: 92)
                    .padding(6)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )

                Button {
                    onScanned(manualPayload)
                    dismiss()
                } label: {
                    Text(L10n.tr("scanner.import_button", default: "导入配对"))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 10)
                        .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                }
                .disabled(manualPayload.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

                Spacer()
            }
            .padding(16)
            .background(SemanticColors.surfacePrimary)
            .navigationTitle(L10n.tr("scanner.title", default: "扫码配对"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.tr("scanner.close", default: "关闭")) { dismiss() }
                }
            }
        }
    }

    private var simulatorPlaceholder: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(L10n.tr("scanner.sim.title", default: "模拟器相机占位"), systemImage: "camera.viewfinder")
                .font(.subheadline.weight(.medium))
            Text(L10n.tr("scanner.sim.body", default: "请在下方粘贴配对链接或 JSON。"))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
    }

    private var cameraDeniedBanner: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(L10n.tr("scanner.denied.title", default: "需要相机权限"))
                .font(.subheadline.weight(.semibold))
            Text(L10n.tr("scanner.denied.body", default: "请在系统设置允许相机访问，或粘贴配对信息。"))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
    }
}

private final class CaptureSessionWorker: NSObject, AVCaptureMetadataOutputObjectsDelegate, @unchecked Sendable {
    private let queue = DispatchQueue(label: "com.asecond.tokenvane.qr-capture")
    private let session = AVCaptureSession()
    private let onDetect: @Sendable (String) -> Void
    private let onFailure: @Sendable (String) -> Void
    private var didConfigure = false
    private var didEmit = false

    init(
        onDetect: @escaping @Sendable (String) -> Void,
        onFailure: @escaping @Sendable (String) -> Void
    ) {
        self.onDetect = onDetect
        self.onFailure = onFailure
        super.init()
    }

    @MainActor
    func makePreviewLayer(bounds: CGRect) -> AVCaptureVideoPreviewLayer {
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        layer.frame = bounds
        return layer
    }

    func start() {
        let inputErrMsg = L10n.tr("scanner.error.input", default: "相机初始化受限")
        let outputErrMsg = L10n.tr("scanner.error.output", default: "二维码识别初始化受限")
        queue.async { [self] in
            if !didConfigure {
                guard let device = AVCaptureDevice.default(for: .video),
                      let input = try? AVCaptureDeviceInput(device: device),
                      session.canAddInput(input) else {
                    Task { @MainActor in
                        self.onFailure(inputErrMsg)
                    }
                    return
                }
                session.addInput(input)

                let output = AVCaptureMetadataOutput()
                guard session.canAddOutput(output) else {
                    Task { @MainActor in
                        self.onFailure(outputErrMsg)
                    }
                    return
                }
                session.addOutput(output)
                output.setMetadataObjectsDelegate(self, queue: queue)
                if output.availableMetadataObjectTypes.contains(.qr) {
                    output.metadataObjectTypes = [.qr]
                }
                didConfigure = true
            }
            if !session.isRunning {
                session.startRunning()
            }
        }
    }

    func stop() {
        queue.async { [self] in
            if session.isRunning {
                session.stopRunning()
            }
        }
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !didEmit,
              let readable = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let value = readable.stringValue else {
            return
        }
        didEmit = true
        session.stopRunning()
        Task { @MainActor in
            self.onDetect(value)
        }
    }
}

private struct CameraQRPreview: UIViewControllerRepresentable {
    let onDetect: @MainActor (String) -> Void
    let onFailure: @MainActor (String) -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let vc = ScannerController()
        vc.onDetect = onDetect
        vc.onFailure = onFailure
        return vc
    }

    func updateUIViewController(_ uiViewController: ScannerController, context: Context) {}

    final class ScannerController: UIViewController {
        var onDetect: (@MainActor (String) -> Void)?
        var onFailure: (@MainActor (String) -> Void)?
        private var worker: CaptureSessionWorker?
        private var previewLayer: AVCaptureVideoPreviewLayer?

        override func viewDidLoad() {
            super.viewDidLoad()
            let worker = CaptureSessionWorker(
                onDetect: { [weak self] code in
                    Task { @MainActor [weak self] in
                        self?.onDetect?(code)
                    }
                },
                onFailure: { [weak self] msg in
                    Task { @MainActor [weak self] in
                        self?.onFailure?(msg)
                    }
                }
            )
            let layer = worker.makePreviewLayer(bounds: view.bounds)
            view.layer.addSublayer(layer)
            self.worker = worker
            self.previewLayer = layer
            worker.start()
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            previewLayer?.frame = view.bounds
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            worker?.stop()
        }
    }
}
