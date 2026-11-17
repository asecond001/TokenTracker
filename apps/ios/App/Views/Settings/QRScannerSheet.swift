import AVFoundation
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

                Text("或手动粘贴桌面端导出的配对 JSON 串")
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
                    Text("确认导入配对信息")
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
            .navigationTitle("扫码配对")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("关闭") { dismiss() }
                }
            }
        }
    }

    private var simulatorPlaceholder: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label("模拟器环境无物理摄像头", systemImage: "camera.viewfinder")
                .font(.subheadline.weight(.medium))
            Text("真机运行将直接打开后置相机扫描二维码；请在下方直接粘贴配对 JSON 进行导入。")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
    }

    private var cameraDeniedBanner: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("未授予相机权限")
                .font(.subheadline.weight(.semibold))
            Text("请在系统设置中允许 TokenVane 访问相机，或在下方直接粘贴配对 JSON。")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
    }
}

private struct CameraQRPreview: UIViewControllerRepresentable {
    let onDetect: (String) -> Void
    let onFailure: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let vc = ScannerController()
        vc.onDetect = onDetect
        vc.onFailure = onFailure
        return vc
    }

    func updateUIViewController(_ uiViewController: ScannerController, context: Context) {}

    final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
        var onDetect: ((String) -> Void)?
        var onFailure: ((String) -> Void)?
        private var session: AVCaptureSession?
        private var previewLayer: AVCaptureVideoPreviewLayer?
        private var didEmit = false

        override func viewDidLoad() {
            super.viewDidLoad()
            let session = AVCaptureSession()
            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input) else {
                onFailure?("无法初始化摄像头输入设备")
                return
            }
            session.addInput(input)

            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else {
                onFailure?("无法初始化二维码识别输出")
                return
            }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: DispatchQueue.main)
            if output.availableMetadataObjectTypes.contains(.qr) {
                output.metadataObjectTypes = [.qr]
            }

            let layer = AVCaptureVideoPreviewLayer(session: session)
            layer.videoGravity = .resizeAspectFill
            layer.frame = view.bounds
            view.layer.addSublayer(layer)

            self.session = session
            self.previewLayer = layer
            DispatchQueue.global(qos: .userInitiated).async {
                session.startRunning()
            }
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            previewLayer?.frame = view.bounds
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            let cap = session
            DispatchQueue.global(qos: .userInitiated).async {
                cap?.stopRunning()
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
            session?.stopRunning()
            onDetect?(value)
        }
    }
}
