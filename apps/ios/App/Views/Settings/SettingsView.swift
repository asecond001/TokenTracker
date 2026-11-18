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
    @State private var resetNotifications = true

    @State private var showingQRScanner = false
    @State private var showingResetAlert = false
    @State private var statusBanner: String?
    @State private var didHydratePreferences = false
    @State private var hydratedSyncRevision: Int?
    @State private var scannedPairingRaw: String?

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
            resetNotifications = store.resetNotificationsEnabled
            // A deep link may have failed to parse before this view existed.
            if let message = store.pairingStatusMessage {
                statusBanner = message
                store.pairingStatusMessage = nil
            }
        }
        .onChange(of: store.bootstrap) { _, _ in
            hydrateFromStore()
        }
        .onChange(of: store.syncConfigRevision) { _, _ in
            hydrateFromStore()
        }
        .onChange(of: store.pairingStatusMessage) { _, message in
            if let message {
                statusBanner = message
                store.pairingStatusMessage = nil
            }
        }
        .sheet(isPresented: $showingQRScanner, onDismiss: {
            // Stage only after the scanner is gone so the root confirmation alert can present.
            if let raw = scannedPairingRaw {
                scannedPairingRaw = nil
                handlePairingRaw(raw)
            }
        }) {
            QRScannerSheet { raw in
                scannedPairingRaw = raw
            }
        }
        .alert(L10n.tr("settings.sync.reset_title", default: "确认重置本地缓存？"), isPresented: $showingResetAlert) {
            Button(L10n.tr("pairing.alert.cancel", default: "取消"), role: .cancel) {}
            Button(L10n.tr("settings.sync.reset_confirm", default: "清空并重拉"), role: .destructive) {
                Task {
                    await store.resetSyncCache()
                    statusBanner = L10n.tr("settings.sync.reset_done", default: "已重置本地缓存")
                }
            }
        } message: {
            Text(L10n.tr("settings.sync.reset_message", default: "将清除已同步设备记录并从头拉取。"))
        }
    }

    private var pairingActionsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(L10n.tr("settings.pair.title", default: "快捷配对"))
                .font(.subheadline.weight(.semibold))

            HStack(spacing: 10) {
                Button {
                    showingQRScanner = true
                } label: {
                    Label(L10n.tr("settings.pair.scan_qr", default: "扫码配对"), systemImage: "qrcode.viewfinder")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 9)
                        .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                }
                .accessibilityIdentifier("settings.scanQRButton")

                Button {
                    if let paste = UIPasteboard.general.string {
                        handlePairingRaw(paste)
                    } else {
                        statusBanner = L10n.tr("settings.pair.clipboard_empty", default: "剪贴板为空")
                    }
                } label: {
                    Label(L10n.tr("settings.pair.paste", default: "剪贴板导入"), systemImage: "doc.on.clipboard")
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
            Toggle(L10n.tr("settings.sync.enable", default: "启用云同步"), isOn: $syncEnabled)
                .font(.subheadline.weight(.medium))

            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.tr("settings.sync.worker_url", default: "Worker URL"))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                TextField("https://tokenvane-sync.workers.dev", text: $workerURL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .font(.subheadline.monospaced())
                    .padding(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
                    .accessibilityIdentifier("settings.workerURLField")
            }

            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.tr("settings.sync.token", default: "Sync Token（Keychain）"))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                SecureField(L10n.tr("settings.sync.token_placeholder", default: "输入 Sync Token"), text: $bearerToken)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.subheadline.monospaced())
                    .padding(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
            }

            Toggle(L10n.tr("settings.sync.e2ee_enable", default: "端到端加密（E2EE）"), isOn: $e2eeEnabled)
                .font(.subheadline.weight(.medium))

            if e2eeEnabled {
                VStack(alignment: .leading, spacing: 4) {
                    Text(L10n.tr("settings.sync.e2ee_passphrase", default: "E2EE 口令（Keychain）"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    SecureField(L10n.tr("settings.sync.e2ee_placeholder", default: "输入解密口令"), text: $e2eePassphrase)
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
                    statusBanner = ok
                        ? L10n.tr("settings.sync.saved", default: "配置已保存")
                        : (store.lastErrorMessage ?? L10n.tr("pairing.status.save_failed", default: "保存失败"))
                }
            } label: {
                Text(L10n.tr("settings.sync.save", default: "保存并连接"))
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
            Text(L10n.tr("settings.diag.title", default: "同步状态"))
                .font(.subheadline.weight(.semibold))

            diagnosticRow(
                title: L10n.tr("settings.diag.watermark", default: "同步水位"),
                value: TokenVaneFormatters.formatWatermarkTimestamp(cfg?.lastPullWatermarkMs)
            )
            diagnosticRow(
                title: L10n.tr("settings.diag.devices", default: "已同步终端"),
                value: L10n.trFormat("overview.devices.count", default: "%lld 台", Int64(syncedCount))
            )
            diagnosticRow(
                title: L10n.tr("settings.diag.last_sync", default: "最近同步"),
                value: TokenVaneFormatters.formatRelativeSync(atMs: cfg?.lastSyncAtMs)
            )

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
                            statusBanner = L10n.trFormat("settings.diag.health_ok", default: "/healthz 正常: %@", txt)
                        case .failure(let err):
                            statusBanner = L10n.trFormat("settings.diag.health_fail", default: "/healthz 异常: %@", err.localizedDescription)
                        }
                    }
                } label: {
                    Text(L10n.tr("settings.diag.test_health", default: "健康检查"))
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
                        statusBanner = L10n.tr("settings.sync.now_done", default: "同步完成")
                    }
                } label: {
                    Text(L10n.tr("settings.sync.now", default: "立即同步"))
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
                Text(L10n.tr("settings.sync.reset_button", default: "重置同步缓存"))
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
            Text(L10n.tr("settings.prefs.title", default: "偏好设置"))
                .font(.subheadline.weight(.semibold))

            HStack {
                Text(L10n.tr("settings.prefs.currency", default: "币种"))
                    .font(.subheadline)
                Spacer()
                Picker(L10n.tr("settings.prefs.currency", default: "币种"), selection: Binding(
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

            Toggle(L10n.tr("settings.prefs.auto_rate", default: "自动汇率 (12h)"), isOn: $autoExchangeRate)
                .font(.subheadline)
                .onChange(of: autoExchangeRate) { _, val in
                    guard val != store.bootstrap?.preferences.exchangeRateAuto else { return }
                    Task {
                        await store.setAutoExchangeRate(val)
                        manualExchangeRate = store.manualRateDraft
                    }
                }

            if autoExchangeRate {
                HStack(spacing: 8) {
                    Text(L10n.tr("settings.prefs.rate_label", default: "USD → CNY"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(store.usdToCnyRate)
                        .font(.subheadline.monospacedDigit().weight(.medium))
                        .tabularMetrics()
                    Spacer()
                    Button(L10n.tr("settings.prefs.rate_fetch", default: "更新汇率")) {
                        Task {
                            if let rate = await store.syncExchangeRateNow() {
                                let formatted = String(format: "%.4f", rate)
                                statusBanner = L10n.trFormat("settings.prefs.rate_updated", default: "汇率已更新: %@", formatted)
                            } else {
                                statusBanner = L10n.tr("settings.prefs.rate_failed", default: "汇率获取异常")
                            }
                        }
                    }
                    .font(.caption.weight(.medium))
                }
            } else {
                HStack(spacing: 8) {
                    Text(L10n.tr("settings.prefs.rate_label", default: "USD → CNY"))
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
                    Button(L10n.tr("settings.prefs.rate_apply", default: "应用")) {
                        Task {
                            if await store.applyManualExchangeRate(manualExchangeRate) {
                                manualExchangeRate = store.manualRateDraft
                                statusBanner = L10n.trFormat("settings.prefs.rate_updated", default: "汇率已更新: %@", manualExchangeRate)
                            } else {
                                statusBanner = L10n.tr("settings.prefs.rate_invalid", default: "请输入 0.1 ~ 100 之间的有效汇率")
                            }
                        }
                    }
                    .font(.caption.weight(.medium))
                    Spacer()
                }
            }

            HStack {
                Text(L10n.tr("settings.prefs.theme", default: "外观"))
                    .font(.subheadline)
                Spacer()
                Picker(L10n.tr("settings.prefs.theme", default: "外观"), selection: $selectedTheme) {
                    Text(L10n.tr("settings.prefs.theme.system", default: "系统")).tag("system")
                    Text(L10n.tr("settings.prefs.theme.light", default: "浅色")).tag("light")
                    Text(L10n.tr("settings.prefs.theme.dark", default: "深色")).tag("dark")
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 220)
                .onChange(of: selectedTheme) { _, val in
                    guard val != store.bootstrap?.preferences.theme else { return }
                    Task { await store.applyPreferencesPatch(MobilePreferencesPatchDTO(theme: val)) }
                }
            }

            Toggle(L10n.tr("settings.prefs.reset_notifications", default: "配额重置通知"), isOn: $resetNotifications)
                .font(.subheadline)
                .onChange(of: resetNotifications) { _, val in
                    store.resetNotificationsEnabled = val
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
        guard let boot = store.bootstrap else { return }
        if !didHydratePreferences {
            didHydratePreferences = true
            manualExchangeRate = store.manualRateDraft
            autoExchangeRate = boot.preferences.exchangeRateAuto
            selectedTheme = boot.preferences.theme
        }
        // Re-hydrate sync fields whenever the config was saved elsewhere (deep link, QR,
        // clipboard), so a later tap on "save" never writes stale values over a new pairing.
        if hydratedSyncRevision != store.syncConfigRevision {
            hydratedSyncRevision = store.syncConfigRevision
            syncEnabled = boot.syncConfig.enabled
            workerURL = boot.syncConfig.workerUrl ?? ""
            e2eeEnabled = boot.syncConfig.e2eeEnabled
            bearerToken = store.loadSecretMasked(account: .syncBearerToken)
            e2eePassphrase = store.loadSecretMasked(account: .e2eePassphrase)
        }
    }

    private func handlePairingRaw(_ raw: String) {
        if !store.stagePairing(raw: raw), let message = store.pairingStatusMessage {
            statusBanner = message
            store.pairingStatusMessage = nil
        }
    }
}
