import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI
import UIKit

private enum SyncSetupMode: String, CaseIterable, Identifiable {
    case connectExisting
    case provisionNew

    var id: String { rawValue }

    var label: String {
        switch self {
        case .connectExisting: return L10n.tr("settings.sync.mode.connect", default: "连接已有 Worker")
        case .provisionNew: return L10n.tr("settings.sync.mode.provision", default: "一键部署 Worker")
        }
    }
}

public struct SettingsView: View {
    @Bindable var store: AppStore

    @State private var setupMode: SyncSetupMode = .connectExisting
    @State private var syncEnabled = false
    @State private var workerURL = ""
    @State private var bearerToken = ""
    @State private var e2eeEnabled = false
    @State private var e2eePassphrase = ""

    // Cloudflare Provisioning inputs
    @State private var cfApiToken = ""
    @State private var cfAccountId = ""
    @State private var workerName = "tokenvane-sync-worker"
    @State private var provisionE2ee = true
    @State private var provisionPassphrase = ""
    @State private var provisionBearer = ""
    @State private var completedProvisionURL: String?

    // Preferences & Workspace inputs
    @State private var deviceDisplayName = ""
    @State private var manualExchangeRate = "7.25"
    @State private var autoExchangeRate = true
    @State private var autoSyncPricing = true
    @State private var selectedTheme = "system"
    @State private var bgMode = "none"
    @State private var bgPresetId = "aurora_slate"
    @State private var bgOpacity = 0.18
    @State private var resetNotifications = true

    @State private var showingPairingQR = false
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
                currencyAndCatalogsSection
                HairlineDivider(insetLeading: 16)
                appearanceAndBackdropSection
            }
            .padding(.bottom, 32)
        }
        .background(Color.clear)
        .onAppear {
            hydrateFromStore()
            resetNotifications = store.resetNotificationsEnabled
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

            if let pairingURI = store.latestPairingURI {
                DisclosureGroup(isExpanded: $showingPairingQR) {
                    VStack(spacing: 10) {
                        if let qrImage = Self.generateQRImage(from: pairingURI) {
                            Image(uiImage: qrImage)
                                .interpolation(.none)
                                .resizable()
                                .scaledToFit()
                                .frame(width: 170, height: 170)
                                .padding(10)
                                .background(Color.white, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        }

                        Button {
                            UIPasteboard.general.string = pairingURI
                            statusBanner = L10n.tr("settings.pair.uri_copied", default: "配对链接已复制到剪贴板")
                        } label: {
                            Label(L10n.tr("settings.pair.copy_uri", default: "复制配对链接"), systemImage: "link")
                                .font(.caption.weight(.medium))
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 8)
                                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(.top, 8)
                } label: {
                    Label(L10n.tr("settings.pair.share_qr", default: "导出本端配对二维码（供其他设备扫码）"), systemImage: "qrcode")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                }
                .padding(.top, 4)
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
            Picker(L10n.tr("settings.sync.mode_picker", default: "配置方式"), selection: $setupMode) {
                ForEach(SyncSetupMode.allCases) { mode in
                    Text(mode.label).tag(mode)
                }
            }
            .pickerStyle(.segmented)

            if setupMode == .connectExisting {
                connectExistingWorkerFields
            } else {
                provisionCloudflareWorkerFields
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    @ViewBuilder
    private var connectExistingWorkerFields: some View {
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

    @ViewBuilder
    private var provisionCloudflareWorkerFields: some View {
        Text(L10n.tr("provision.desc", default: "在手机端直接通过 Cloudflare API Token 部署 SyncHubDO Worker，无需电脑终端。"))
            .font(.caption)
            .foregroundStyle(.secondary)

        VStack(alignment: .leading, spacing: 4) {
            Text(L10n.tr("provision.cf_token", default: "Cloudflare API Token（Workers Scripts:Edit）"))
                .font(.caption2)
                .foregroundStyle(.secondary)
            SecureField(L10n.tr("provision.cf_token_placeholder", default: "输入 Cloudflare API 令牌（存 Keychain）"), text: $cfApiToken)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.subheadline.monospaced())
                .padding(8)
                .overlay(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                )
        }

        VStack(alignment: .leading, spacing: 4) {
            Text(L10n.tr("provision.worker_name", default: "Worker 名称"))
                .font(.caption2)
                .foregroundStyle(.secondary)
            TextField("tokenvane-sync-worker", text: $workerName)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.subheadline.monospaced())
                .padding(8)
                .overlay(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                )
        }

        VStack(alignment: .leading, spacing: 4) {
            Text(L10n.tr("provision.account_id", default: "Account ID（可选）"))
                .font(.caption2)
                .foregroundStyle(.secondary)
            TextField(L10n.tr("provision.account_id_placeholder", default: "留空自动识别第一个账户"), text: $cfAccountId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.subheadline.monospaced())
                .padding(8)
                .overlay(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                )
        }

        Toggle(L10n.tr("provision.e2ee", default: "启用端到端加密（E2EE）"), isOn: $provisionE2ee)
            .font(.subheadline.weight(.medium))

        if provisionE2ee {
            SecureField(L10n.tr("provision.passphrase_placeholder", default: "E2EE 口令（留空自动生成）"), text: $provisionPassphrase)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.subheadline.monospaced())
                .padding(8)
                .overlay(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                )
        }

        SecureField(L10n.tr("provision.bearer_placeholder", default: "Sync Token（留空自动生成随机密钥）"), text: $provisionBearer)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .font(.subheadline.monospaced())
            .padding(8)
            .overlay(
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .strokeBorder(SemanticColors.hairline, lineWidth: 1)
            )

        if store.isProvisioning || store.provisionStep != nil {
            provisionProgressCard
        }

        Button {
            Task {
                if let outcome = await store.provisionWorker(
                    workerName: workerName,
                    cfApiToken: cfApiToken,
                    cfAccountId: cfAccountId,
                    bearerToken: provisionBearer,
                    e2eeEnabled: provisionE2ee,
                    e2eePassphrase: provisionPassphrase
                ) {
                    completedProvisionURL = outcome.workerURL
                    showingPairingQR = true
                    statusBanner = L10n.trFormat("provision.done", default: "部署并连接成功: %@", outcome.workerURL)
                } else {
                    statusBanner = store.lastErrorMessage ?? L10n.tr("provision.failed", default: "部署失败")
                }
            }
        } label: {
            HStack(spacing: 8) {
                if store.isProvisioning {
                    ProgressView()
                        .tint(.white)
                        .controlSize(.small)
                }
                Text(
                    store.isProvisioning
                        ? L10n.tr("provision.running", default: "正在部署...")
                        : L10n.tr("provision.submit", default: "一键部署并连接")
                )
                .font(.subheadline.weight(.semibold))
            }
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 10)
            .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .disabled(store.isProvisioning)
    }

    private var provisionProgressCard: some View {
        let steps = CloudSyncProvisionStep.allCases
        let currentIndex = store.provisionStep.flatMap { steps.firstIndex(of: $0) } ?? -1
        let allDone = (!store.isProvisioning && completedProvisionURL != nil)

        return VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(steps.enumerated()), id: \.element) { idx, step in
                HStack(spacing: 8) {
                    if allDone || idx < currentIndex {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.caption)
                            .foregroundStyle(SemanticColors.statusLive)
                    } else if store.isProvisioning && idx == currentIndex {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Circle()
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                            .frame(width: 12, height: 12)
                    }
                    Text(step.label)
                        .font(.caption)
                        .foregroundStyle(idx <= currentIndex || allDone ? .primary : .secondary)
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
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

    private var currencyAndCatalogsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.tr("settings.prefs.title", default: "偏好与在线目录"))
                .font(.subheadline.weight(.semibold))

            HStack(spacing: 8) {
                Text(L10n.tr("settings.prefs.device_name", default: "本机名称"))
                    .font(.subheadline)
                Spacer()
                TextField("iPhone", text: $deviceDisplayName)
                    .multilineTextAlignment(.trailing)
                    .font(.subheadline)
                    .frame(maxWidth: 160)
                    .padding(.vertical, 4)
                    .padding(.horizontal, 6)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .strokeBorder(SemanticColors.hairline, lineWidth: 1)
                    )
                Button(L10n.tr("common.save", default: "保存")) {
                    Task {
                        await store.updateDeviceDisplayName(deviceDisplayName)
                        statusBanner = L10n.tr("settings.prefs.device_saved", default: "设备名称已更新")
                    }
                }
                .font(.caption.weight(.medium))
            }

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

            Toggle(L10n.tr("settings.prefs.auto_catalogs", default: "自动同步模型单价与订阅目录 (12h)"), isOn: $autoSyncPricing)
                .font(.subheadline)
                .onChange(of: autoSyncPricing) { _, val in
                    guard val != store.bootstrap?.preferences.autoSyncPricing else { return }
                    Task { await store.applyPreferencesPatch(MobilePreferencesPatchDTO(autoSyncPricing: val)) }
                }

            Button {
                Task {
                    if let summary = await store.syncAllCatalogsNow() {
                        statusBanner = L10n.trFormat(
                            "settings.catalogs.updated",
                            default: "目录更新完成：单价 %lld 条 · 订阅 %lld 条",
                            Int64(summary.pricingCount ?? 0),
                            Int64(summary.subscriptionCount ?? 0)
                        )
                    }
                }
            } label: {
                HStack(spacing: 8) {
                    if store.isSyncingCatalogs {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Image(systemName: "arrow.clockwise.circle")
                            .font(.caption)
                    }
                    Text(L10n.tr("settings.catalogs.sync_now", default: "立即刷新汇率、模型单价与订阅目录"))
                        .font(.caption.weight(.medium))
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(store.isSyncingCatalogs)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private var appearanceAndBackdropSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.tr("settings.appearance.title", default: "外观与背景氛围"))
                .font(.subheadline.weight(.semibold))

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

            HStack {
                Text(L10n.tr("settings.backdrop.mode", default: "背景模式"))
                    .font(.subheadline)
                Spacer()
                Picker(L10n.tr("settings.backdrop.mode", default: "背景模式"), selection: $bgMode) {
                    Text(L10n.tr("settings.backdrop.none", default: "纯净底色")).tag("none")
                    Text(L10n.tr("settings.backdrop.gradient", default: "氛围光影")).tag("gradient")
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 190)
                .onChange(of: bgMode) { _, val in
                    guard val != store.bootstrap?.preferences.bgMode else { return }
                    Task { await store.setBackdrop(mode: val, presetId: bgPresetId, opacity: bgOpacity) }
                }
            }

            if bgMode == "gradient" {
                LazyVGrid(
                    columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)],
                    spacing: 8
                ) {
                    ForEach(BackdropPresetOption.allPresets) { preset in
                        let selected = bgPresetId == preset.id
                        Button {
                            bgPresetId = preset.id
                            Task { await store.setBackdrop(mode: "gradient", presetId: preset.id, opacity: bgOpacity) }
                        } label: {
                            HStack(spacing: 8) {
                                LinearGradient(
                                    colors: [preset.darkStops[0], preset.darkStops[1]],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                )
                                .frame(width: 24, height: 24)
                                .clipShape(RoundedRectangle(cornerRadius: 5, style: .continuous))
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(preset.title)
                                        .font(.caption.weight(.medium))
                                        .foregroundStyle(.primary)
                                    Text(preset.subtitle)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                                Spacer(minLength: 0)
                            }
                            .padding(8)
                            .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: 7, style: .continuous)
                                    .strokeBorder(
                                        selected ? SemanticColors.accentIndigo : SemanticColors.hairline,
                                        lineWidth: selected ? 1.5 : 1
                                    )
                            )
                        }
                        .buttonStyle(.plain)
                    }
                }

                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(L10n.tr("settings.backdrop.opacity", default: "光影浓度"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Spacer()
                        Text("\(Int((bgOpacity * 100).rounded()))%")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    Slider(value: $bgOpacity, in: 0.05...0.55, step: 0.02) { editing in
                        if !editing {
                            Task { await store.setBackdrop(mode: bgMode, presetId: bgPresetId, opacity: bgOpacity) }
                        }
                    }
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
            autoSyncPricing = boot.preferences.autoSyncPricing
            selectedTheme = boot.preferences.theme
            deviceDisplayName = boot.preferences.deviceDisplayName
            bgMode = boot.preferences.bgMode
            bgPresetId = boot.preferences.bgPresetId
            bgOpacity = boot.preferences.bgOpacity
            cfApiToken = store.loadSecretMasked(account: .cloudflareApiToken)
        }
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

    private static func generateQRImage(from text: String) -> UIImage? {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }
}
