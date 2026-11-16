import SwiftUI
import UIKit

public struct SettingsView: View {
    @Bindable var store: AppStore

    @State private var syncEnabled = false
    @State private var workerURL = ""
    @State private var bearerToken = ""
    @State private var e2eeEnabled = false
    @State private var e2eePassphrase = ""
    @State private var manualExchangeRate = "7.25"
    @State private var autoExchangeRate = true
    @State private var selectedTheme = "system"

    @State private var showingQRScanner = false
    @State private var showingResetAlert = false
    @State private var statusBanner: String?
    @State private var didHydrateFields = false

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                pairingActionsSection
                HairlineDivider(insetLeading: 16)
                syncConfigurationSection
                HairlineDivider(insetLeading: 16)
                diagnosticsSection
                HairlineDivider(insetLeading: 16)
                currencyAndAppearanceSection
            }
            .padding(.bottom, 32)
        }
        .background(SemanticColors.surfacePrimary)
        .onAppear {
            hydrateFromStore()
        }
        .onChange(of: store.bootstrap) { _, _ in
            hydrateFromStore()
        }
        .sheet(isPresented: $showingQRScanner) {
            QRScannerSheet { raw in
                Task { await handlePairingRaw(raw) }
            }
        }
        .alert("确认重置本地同步缓存？", isPresented: $showingResetAlert) {
            Button("取消", role: .cancel) {}
            Button("清空并重拉", role: .destructive) {
                Task {
                    await store.resetSyncCache()
                    statusBanner = "已清空本地远端缓存与水位线"
                }
            }
        } message: {
            Text("将删除所有已同步的远端设备聚合数据与订阅快照，并从 watermark=0 重新全量拉取。")
        }
    }

    private var pairingActionsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("桌面端一键配对")
                .font(.subheadline.weight(.semibold))
            Text("在桌面端「系统与云同步」生成配对串，可通过相机扫码或剪贴板直接导入 Worker URL 与 Bearer Token（密钥将写入 iOS Keychain）。")
                .font(.caption)
                .foregroundStyle(.secondary)

            HStack(spacing: 10) {
                Button {
                    showingQRScanner = true
                } label: {
                    Label("扫码一键配对", systemImage: "qrcode.viewfinder")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 9)
                        .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                }
                .accessibilityIdentifier("settings.scanQRButton")

                Button {
                    if let paste = UIPasteboard.general.string {
                        Task { await handlePairingRaw(paste) }
                    } else {
                        statusBanner = "剪贴板为空"
                    }
                } label: {
                    Label("从剪贴板导入", systemImage: "doc.on.clipboard")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.primary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 9)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }
            }

            if let statusBanner {
                Text(statusBanner)
                    .font(.caption)
                    .foregroundStyle(SemanticColors.accentIndigo)
                    .padding(.top, 2)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var syncConfigurationSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Toggle("启用 Cloudflare 云同步", isOn: $syncEnabled)
                .font(.subheadline.weight(.medium))

            VStack(alignment: .leading, spacing: 4) {
                Text("Worker URL")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                TextField("https://tokentracker-sync.workers.dev", text: $workerURL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .font(.subheadline.monospaced())
                    .padding(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Bearer Sync Token（Keychain 安全存储）")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                SecureField("粘贴 Sync Token", text: $bearerToken)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.subheadline.monospaced())
                    .padding(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
            }

            Toggle("端到端加密（E2EE AES-256-GCM）", isOn: $e2eeEnabled)
                .font(.subheadline.weight(.medium))

            if e2eeEnabled {
                VStack(alignment: .leading, spacing: 4) {
                    Text("E2EE 解密口令（Keychain 安全存储）")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    SecureField("输入与桌面端一致的口令", text: $e2eePassphrase)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.subheadline.monospaced())
                        .padding(8)
                        .overlay(
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }
            }

            Button {
                Task {
                    let ok = await store.saveSyncSettings(
                        enabled: syncEnabled,
                        workerURL: workerURL,
                        bearerToken: bearerToken,
                        e2eeEnabled: e2eeEnabled,
                        e2eePassphrase: e2eePassphrase
                    )
                    statusBanner = ok ? "同步配置已保存" : (store.lastErrorMessage ?? "保存失败")
                }
            } label: {
                Text("保存并连接云同步")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.white)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var diagnosticsSection: some View {
        let cfg = store.bootstrap?.syncConfig
        let syncedCount = store.bootstrap?.syncedDevices.count ?? 0

        return VStack(alignment: .leading, spacing: 10) {
            Text("同步诊断与水位线")
                .font(.subheadline.weight(.semibold))

            diagnosticRow(title: "Pull 水位线", value: "\(cfg?.lastPullWatermarkMs ?? 0)")
            diagnosticRow(title: "已同步终端", value: "\(syncedCount) 台")
            diagnosticRow(title: "最近同步时间", value: TokenTrackerFormatters.formatRelativeSync(atMs: cfg?.lastSyncAtMs))

            if let err = store.lastErrorMessage ?? cfg?.lastSyncError {
                Text(err)
                    .font(.caption)
                    .foregroundStyle(SemanticColors.statusDanger)
            }

            HStack(spacing: 10) {
                Button {
                    Task {
                        let res = await store.testWorkerHealth(url: workerURL, token: bearerToken)
                        switch res {
                        case .success(let txt):
                            statusBanner = "/healthz 正常: \(txt)"
                        case .failure(let err):
                            statusBanner = "/healthz 失败: \(err.localizedDescription)"
                        }
                    }
                } label: {
                    Text("测试 /healthz")
                        .font(.caption.weight(.medium))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .overlay(
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }

                Button {
                    Task {
                        await store.triggerManualSync()
                        statusBanner = "已完成增量拉取"
                    }
                } label: {
                    Text("立即增量同步")
                        .font(.caption.weight(.medium))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .overlay(
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                        )
                }
            }

            Button(role: .destructive) {
                showingResetAlert = true
            } label: {
                Text("清空本地同步缓存并重拉")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(SemanticColors.statusDanger)
            }
            .padding(.top, 2)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var currencyAndAppearanceSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("币种、汇率与外观")
                .font(.subheadline.weight(.semibold))

            HStack {
                Text("显示币种")
                    .font(.subheadline)
                Spacer()
                Picker("币种", selection: Binding(
                    get: { store.currency },
                    set: { next in
                        Task { await store.applyPreferencesPatch(MobilePreferencesPatchDTO(currency: next)) }
                    }
                )) {
                    Text("USD ($)").tag("USD")
                    Text("CNY (¥)").tag("CNY")
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 180)
            }

            Toggle("自动同步 USD → CNY 汇率", isOn: $autoExchangeRate)
                .font(.subheadline)
                .onChange(of: autoExchangeRate) { _, val in
                    Task { await store.applyPreferencesPatch(MobilePreferencesPatchDTO(exchangeRateAuto: val)) }
                }

            HStack(spacing: 8) {
                Text("USD → CNY")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField("7.25", text: $manualExchangeRate)
                    .keyboardType(.decimalPad)
                    .font(.subheadline.monospacedDigit())
                    .padding(6)
                    .frame(width: 90)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
                Button("应用") {
                    Task {
                        await store.applyPreferencesPatch(MobilePreferencesPatchDTO(usdToCnyRate: manualExchangeRate))
                        statusBanner = "已更新汇率 \(manualExchangeRate)"
                    }
                }
                .font(.caption.weight(.medium))
                Spacer()
                Button("在线获取") {
                    Task {
                        if let rate = await store.syncExchangeRateNow() {
                            manualExchangeRate = String(format: "%.4f", rate)
                            statusBanner = "汇率已更新: \(manualExchangeRate)"
                        } else {
                            statusBanner = "获取在线汇率失败"
                        }
                    }
                }
                .font(.caption.weight(.medium))
            }

            HStack {
                Text("界面主题")
                    .font(.subheadline)
                Spacer()
                Picker("主题", selection: $selectedTheme) {
                    Text("跟随系统").tag("system")
                    Text("浅色").tag("light")
                    Text("深色").tag("dark")
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 220)
                .onChange(of: selectedTheme) { _, val in
                    Task { await store.applyPreferencesPatch(MobilePreferencesPatchDTO(theme: val)) }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func diagnosticRow(title: String, value: String) -> some View {
        HStack {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Text(value)
                .font(.caption.weight(.medium))
                .tabularMetrics()
        }
    }

    private func hydrateFromStore() {
        guard !didHydrateFields, let boot = store.bootstrap else { return }
        didHydrateFields = true
        syncEnabled = boot.syncConfig.enabled
        workerURL = boot.syncConfig.workerUrl ?? ""
        e2eeEnabled = boot.syncConfig.e2eeEnabled
        manualExchangeRate = boot.preferences.usdToCnyRate
        autoExchangeRate = boot.preferences.exchangeRateAuto
        selectedTheme = boot.preferences.theme
        bearerToken = store.loadSecretMasked(account: .syncBearerToken)
        e2eePassphrase = store.loadSecretMasked(account: .e2eePassphrase)
    }

    private func handlePairingRaw(_ raw: String) async {
        let res = await store.applyPairingPayload(raw, passphrase: e2eePassphrase)
        switch res {
        case .success(let payload):
            syncEnabled = true
            workerURL = payload.url
            bearerToken = payload.token
            e2eeEnabled = payload.e2ee
            statusBanner = payload.e2ee && e2eePassphrase.isEmpty
                ? "已导入配对配置；请在下方填写 E2EE 口令后点击保存"
                : "配对导入成功"
        case .failure(let err):
            statusBanner = err.localizedDescription
        }
    }
}
