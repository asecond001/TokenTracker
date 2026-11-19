import SwiftUI
import UIKit

private enum AccountModalKind: String, CaseIterable, Identifiable {
    case payg
    case subscription

    var id: String { rawValue }

    var label: String {
        switch self {
        case .payg: return L10n.tr("accounts.kind.payg", default: "按量 API / PAYG")
        case .subscription: return L10n.tr("accounts.kind.subscription", default: "订阅配额计划")
        }
    }
}

private enum ActiveAccountSheet: Identifiable {
    case create
    case editPayg(PaygAccountStatusDTO)
    case editSubscription(SubscriptionPlanStatusDTO)

    var id: String {
        switch self {
        case .create: return "create"
        case .editPayg(let item): return "payg::\(item.account.accountId)"
        case .editSubscription(let item): return "sub::\(item.plan.planId)"
        }
    }
}

public struct QuotasAndModelsView: View {
    @Environment(\.openURL) private var openURL
    @Bindable var store: AppStore
    @State private var activeSheet: ActiveAccountSheet?

    private static let officialBalanceProviders: Set<String> = [
        "kimi_code",
        "deepseek_harness",
        "qcode",
        "minimax_code"
    ]

    public init(store: AppStore) {
        self.store = store
    }

    public var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                accountHubHeader
                HairlineDivider(insetLeading: 16)
                subscriptionsSection
                if !store.localSubscriptions.isEmpty {
                    HairlineDivider(insetLeading: 16)
                    localSubscriptionsSection
                }
                HairlineDivider(insetLeading: 16)
                paygAccountsSection
            }
            .padding(.bottom, 24)
        }
        .refreshable {
            await store.triggerManualSync()
        }
        .background(Color.clear)
        .sheet(item: $activeSheet) { sheet in
            switch sheet {
            case .create:
                AccountEditorSheet(
                    store: store,
                    initialKind: .payg,
                    editingPayg: nil,
                    editingPlan: nil
                )
            case .editPayg(let item):
                AccountEditorSheet(
                    store: store,
                    initialKind: .payg,
                    editingPayg: item.account,
                    editingPlan: nil
                )
            case .editSubscription(let item):
                AccountEditorSheet(
                    store: store,
                    initialKind: .subscription,
                    editingPayg: nil,
                    editingPlan: item.plan
                )
            }
        }
    }

    // MARK: - Account Hub Summary Strip

    private var filteredSubscriptions: [RemoteSubscriptionSnapshotDTO] {
        let all = store.bootstrap?.remoteSubscriptions ?? []
        if let devId = store.selectedDeviceId {
            return all.filter { $0.deviceId == devId }
        }
        return all
    }

    private var filteredRemotePayg: [RemotePaygSnapshotDTO] {
        let all = store.bootstrap?.remotePaygAccounts ?? []
        if let devId = store.selectedDeviceId {
            return all.filter { $0.deviceId == devId }
        }
        return all
    }

    private var uniqueSubscriptionCount: Int {
        let remotePlans = Set(filteredSubscriptions.map { "\($0.deviceId)::\($0.planId)" })
        let localPlans = Set(store.localSubscriptions.map(\.plan.planId))
        return remotePlans.count + localPlans.count
    }

    private var totalMonthlyCommitmentMicroUsd: Int64 {
        var seenRemote = Set<String>()
        var sum: Int64 = 0
        for sub in filteredSubscriptions where sub.monthlyFeeMicroUsd > 0 {
            let key = "\(sub.deviceId)::\(sub.planId)"
            if seenRemote.insert(key).inserted {
                sum += sub.monthlyFeeMicroUsd
            }
        }
        for local in store.localSubscriptions where local.plan.enabled && local.plan.monthlyFeeMicroUsd > 0 {
            sum += local.plan.monthlyFeeMicroUsd
        }
        return sum
    }

    private var accountHubHeader: some View {
        let paygCount = filteredRemotePayg.count + store.paygAccounts.count

        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(L10n.tr("quotas.hub.title", default: "账户与配额中心"))
                        .font(.subheadline.weight(.semibold))
                    Text(
                        L10n.trFormat(
                            "quotas.hub.summary",
                            default: "%lld 项订阅 · %lld 个按量账户 · 月固定 %@",
                            Int64(uniqueSubscriptionCount),
                            Int64(paygCount),
                            TokenVaneFormatters.formatCurrency(
                                totalMonthlyCommitmentMicroUsd,
                                currency: store.currency,
                                rateString: store.usdToCnyRate
                            )
                        )
                    )
                    .font(.caption2)
                    .tabularMetrics()
                    .foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    activeSheet = .create
                } label: {
                    Label(L10n.tr("quotas.hub.add", default: "添加账户"), systemImage: "plus")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(SemanticColors.accentIndigo, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                }
                .accessibilityIdentifier("quotas.addAccountButton")
            }

            if let feedback = store.liveActivityFeedback {
                HStack(spacing: 8) {
                    Text(feedback.message)
                        .font(.caption)
                        .foregroundStyle(SemanticColors.statusWarning)
                    Spacer()
                    if feedback.canOpenSettings,
                       let settingsURL = URL(string: UIApplication.openSettingsURLString) {
                        Button(L10n.tr("live_activity.open_settings", default: "系统设置")) {
                            openURL(settingsURL)
                        }
                        .font(.caption.weight(.semibold))
                    }
                    Button {
                        store.dismissLiveActivityFeedback()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }

            if let error = store.lastErrorMessage {
                Text(error)
                    .font(.caption2)
                    .foregroundStyle(SemanticColors.statusDanger)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
    }

    // MARK: - Synced Remote Subscriptions

    private var groupedSubscriptions: [(deviceName: String, items: [RemoteSubscriptionSnapshotDTO])] {
        let grouped = Dictionary(grouping: filteredSubscriptions, by: \.deviceDisplayName)
        return grouped.keys.sorted().map { key in
            let sortedItems = (grouped[key] ?? []).sorted { $0.usedPercent > $1.usedPercent }
            return (deviceName: key, items: sortedItems)
        }
    }

    private var subscriptionsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(L10n.tr("quotas.section.title", default: "已同步订阅窗口"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
            }

            if groupedSubscriptions.isEmpty {
                Text(L10n.tr("quotas.waiting", default: "订阅窗口待同步，或点击右上角添加本机计划"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 6)
            } else {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(groupedSubscriptions, id: \.deviceName) { group in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(group.deviceName)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            ForEach(group.items) { sub in
                                subscriptionRow(sub)
                                HairlineDivider()
                            }
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func subscriptionRow(_ sub: RemoteSubscriptionSnapshotDTO) -> some View {
        let isPinned = store.pinnedWatchKeys.contains(sub.id)
        let severityColor = TokenVaneFormatters.quotaSeverityColor(usedPercent: sub.usedPercent)

        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Circle()
                    .fill(TokenVaneFormatters.providerColor(sub.providerId))
                    .frame(width: 7, height: 7)
                VStack(alignment: .leading, spacing: 1) {
                    Text("\(sub.planName) · \(sub.windowLabel)")
                        .font(.subheadline.weight(.medium))
                    if sub.monthlyFeeMicroUsd > 0 {
                        Text(
                            L10n.trFormat(
                                "quotas.fee_per_month",
                                default: "%@/月",
                                TokenVaneFormatters.formatCurrency(
                                    sub.monthlyFeeMicroUsd,
                                    currency: store.currency,
                                    rateString: store.usdToCnyRate
                                )
                            )
                        )
                        .font(.caption2)
                        .tabularMetrics()
                        .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Button {
                    Task { await store.toggleLiveActivityWatch(for: sub) }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: isPinned ? "stop.circle.fill" : "dot.radiowaves.left.and.right")
                            .font(.caption2)
                        Text(
                            isPinned
                                ? L10n.tr("quota.unpin_live_activity", default: "停止")
                                : L10n.tr("quota.pin_live_activity", default: "盯盘")
                        )
                        .font(.caption2.weight(.medium))
                    }
                    .foregroundStyle(isPinned ? SemanticColors.statusDanger : SemanticColors.accentIndigo)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .strokeBorder(
                                isPinned ? SemanticColors.statusDanger.opacity(0.4) : SemanticColors.accentIndigo.opacity(0.4),
                                lineWidth: 1
                            )
                    )
                }
                .buttonStyle(.plain)
            }

            HStack {
                LinearProgressStrip(progressPercent: sub.usedPercent, tint: severityColor)
                Text(TokenVaneFormatters.formatPercent(sub.usedPercent))
                    .font(.caption.weight(.semibold))
                    .tabularMetrics()
                    .foregroundStyle(severityColor)
                    .frame(width: 52, alignment: .trailing)
            }

            HStack {
                Text(
                    sub.source == "official_header" || sub.source == "official_api" || sub.source == "official_cli" || sub.source == "official_log_snapshot"
                        ? L10n.tr("overview.quotas.source.official", default: "官方")
                        : L10n.tr("overview.quotas.source.estimated", default: "推算")
                )
                .font(.caption2)
                .foregroundStyle(.secondary)
                Spacer()
                HStack(spacing: 2) {
                    QuotaCountdownText(resetsAtMs: sub.resetsAtMs)
                    Text(L10n.tr("quota.reset_suffix", default: "后重置"))
                }
                .font(.caption2)
                .tabularMetrics()
                .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
    }

    // MARK: - Local Subscriptions (iPhone configured)

    private var localSubscriptionsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.tr("quotas.local.title", default: "本机订阅计划"))
                .font(.subheadline.weight(.semibold))

            ForEach(store.localSubscriptions) { status in
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Circle()
                            .fill(TokenVaneFormatters.providerColor(status.plan.providerId))
                            .frame(width: 7, height: 7)
                        Text(status.plan.planName)
                            .font(.subheadline.weight(.medium))
                        Text(TokenVaneFormatters.formatCurrency(status.plan.monthlyFeeMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))
                            .font(.caption2)
                            .tabularMetrics()
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button(L10n.tr("models.action.edit", default: "编辑")) {
                            activeSheet = .editSubscription(status)
                        }
                        .font(.caption2.weight(.medium))
                        Button(role: .destructive) {
                            Task { _ = await store.deleteSubscriptionPlan(planId: status.plan.planId) }
                        } label: {
                            Image(systemName: "trash")
                                .font(.caption2)
                                .foregroundStyle(SemanticColors.statusDanger)
                        }
                        .buttonStyle(.plain)
                    }

                    Text("本月 \(TokenVaneFormatters.formatCurrency(status.monthCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate)) · \(TokenVaneFormatters.formatCompactTokens(status.monthTokens))")
                        .font(.caption2)
                        .tabularMetrics()
                        .foregroundStyle(.secondary)

                    ForEach(status.windows) { win in
                        let color = TokenVaneFormatters.quotaSeverityColor(usedPercent: win.usedPercent)
                        VStack(alignment: .leading, spacing: 3) {
                            HStack {
                                Text(win.label)
                                    .font(.caption.weight(.medium))
                                Spacer()
                                Text(TokenVaneFormatters.formatPercent(win.usedPercent))
                                    .font(.caption.weight(.semibold))
                                    .tabularMetrics()
                                    .foregroundStyle(color)
                            }
                            LinearProgressStrip(progressPercent: win.usedPercent, tint: color)
                            HStack {
                                Text(win.source == "local_estimation" ? L10n.tr("overview.quotas.source.estimated", default: "推算") : L10n.tr("overview.quotas.source.official", default: "官方"))
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                Spacer()
                                HStack(spacing: 2) {
                                    QuotaCountdownText(resetsAtMs: win.resetsAtMs)
                                    Text(L10n.tr("quota.reset_suffix", default: "后重置"))
                                }
                                .font(.caption2)
                                .tabularMetrics()
                                .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .padding(.vertical, 6)
                HairlineDivider()
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    // MARK: - Pay-as-you-go (Remote + Local)

    private var paygAccountsSection: some View {
        let localList = store.paygAccounts
        let remoteList = filteredRemotePayg

        return VStack(alignment: .leading, spacing: 12) {
            Text(L10n.tr("quotas.payg.title", default: "按量付费 / API 余额"))
                .font(.subheadline.weight(.semibold))

            if localList.isEmpty && remoteList.isEmpty {
                Text(L10n.tr("quotas.payg.empty", default: "暂无按量计费账户，点击右上角添加 API Key 或手工余额追踪"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 6)
            } else {
                ForEach(localList) { status in
                    localPaygRow(status)
                    HairlineDivider()
                }
                ForEach(remoteList) { remote in
                    remotePaygRow(remote)
                    HairlineDivider()
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func localPaygRow(_ status: PaygAccountStatusDTO) -> some View {
        let acct = status.account
        let isRefreshing = store.refreshingPaygAccountIds.contains(acct.accountId)
        let supportsOfficial = Self.officialBalanceProviders.contains(acct.providerId)
        let hasSavedKey = !store.loadSecretMasked(account: .providerApiKey(acct.accountId)).isEmpty
            || !store.loadSecretMasked(account: .providerApiKey(acct.providerId)).isEmpty

        let balanceText: String = {
            if let off = status.officialBalance {
                return formatBalanceAmount(micro: off.totalBalanceMicro, balCurrency: off.currency)
            }
            if let manual = status.remainingManualBalanceMicroUsd {
                return TokenVaneFormatters.formatCurrency(manual, currency: store.currency, rateString: store.usdToCnyRate)
            }
            return "—"
        }()

        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Circle()
                    .fill(TokenVaneFormatters.providerColor(acct.providerId))
                    .frame(width: 7, height: 7)
                VStack(alignment: .leading, spacing: 1) {
                    Text(acct.accountName)
                        .font(.subheadline.weight(.medium))
                    Text(TokenVaneFormatters.providerDisplayName(acct.providerId))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Text(balanceText)
                    .font(.subheadline.weight(.semibold))
                    .tabularMetrics()
            }

            HStack(spacing: 10) {
                Text("今日 \(TokenVaneFormatters.formatCurrency(status.todayCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
                Text("本月 \(TokenVaneFormatters.formatCurrency(status.monthCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
                Spacer()
                if let off = status.officialBalance {
                    Text(TokenVaneFormatters.formatRelativeSync(atMs: off.fetchedAtMs))
                }
            }
            .font(.caption2)
            .tabularMetrics()
            .foregroundStyle(.secondary)

            if let budgetPct = status.budgetUsedPercent, let budgetMicro = acct.monthlyBudgetMicroUsd, budgetMicro > 0 {
                HStack(spacing: 8) {
                    LinearProgressStrip(
                        progressPercent: budgetPct,
                        tint: TokenVaneFormatters.quotaSeverityColor(usedPercent: budgetPct)
                    )
                    Text("\(TokenVaneFormatters.formatPercent(budgetPct, decimals: 0)) / \(TokenVaneFormatters.formatCurrency(budgetMicro, currency: store.currency, rateString: store.usdToCnyRate))")
                        .font(.caption2)
                        .tabularMetrics()
                        .foregroundStyle(.secondary)
                }
            }

            HStack(spacing: 10) {
                if supportsOfficial && hasSavedKey {
                    Button {
                        Task { _ = await store.refreshPaygBalance(acct) }
                    } label: {
                        HStack(spacing: 4) {
                            if isRefreshing {
                                ProgressView()
                                    .controlSize(.mini)
                            } else {
                                Image(systemName: "arrow.clockwise")
                                    .font(.caption2)
                            }
                            Text(L10n.tr("quotas.payg.refresh", default: "刷新余额"))
                                .font(.caption2.weight(.medium))
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                    }
                    .disabled(isRefreshing)
                }

                Spacer()

                Button(L10n.tr("models.action.edit", default: "编辑")) {
                    activeSheet = .editPayg(status)
                }
                .font(.caption2.weight(.medium))

                Button(role: .destructive) {
                    Task { _ = await store.deletePaygAccount(accountId: acct.accountId) }
                } label: {
                    Text(L10n.tr("common.delete", default: "删除"))
                        .font(.caption2)
                        .foregroundStyle(SemanticColors.statusDanger)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func remotePaygRow(_ remote: RemotePaygSnapshotDTO) -> some View {
        let balanceText: String = {
            if let cur = remote.currency {
                return formatBalanceAmount(micro: remote.totalBalanceMicro, balCurrency: cur)
            }
            if let manual = remote.remainingManualBalanceMicroUsd {
                return TokenVaneFormatters.formatCurrency(manual, currency: store.currency, rateString: store.usdToCnyRate)
            }
            return "—"
        }()

        return VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Circle()
                    .fill(TokenVaneFormatters.providerColor(remote.providerId))
                    .frame(width: 7, height: 7)
                VStack(alignment: .leading, spacing: 1) {
                    Text(remote.accountName)
                        .font(.subheadline.weight(.medium))
                    Text("\(remote.deviceDisplayName) · \(TokenVaneFormatters.providerDisplayName(remote.providerId))")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Text(balanceText)
                    .font(.subheadline.weight(.semibold))
                    .tabularMetrics()
            }

            HStack(spacing: 10) {
                Text("今日 \(TokenVaneFormatters.formatCurrency(remote.todayCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
                Text("本月 \(TokenVaneFormatters.formatCurrency(remote.monthCostMicroUsd, currency: store.currency, rateString: store.usdToCnyRate))")
                Spacer()
                if let fetched = remote.fetchedAtMs {
                    Text(TokenVaneFormatters.formatRelativeSync(atMs: fetched))
                }
            }
            .font(.caption2)
            .tabularMetrics()
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    private func formatBalanceAmount(micro: Int64?, balCurrency: String) -> String {
        let upper = balCurrency.uppercased()
        if upper == "CREDITS_UNLIMITED" {
            return "Unlimited Credits"
        }
        guard let micro else { return "—" }
        let value = Double(micro) / 1_000_000.0
        if upper == "CREDITS" {
            return String(format: "%.2f Credits", locale: Locale(identifier: "en_US_POSIX"), value)
        }
        if upper == "CNY" {
            return String(format: "¥%.2f", locale: Locale(identifier: "en_US_POSIX"), value)
        }
        if upper == "USD" {
            return TokenVaneFormatters.formatCurrency(micro, currency: store.currency, rateString: store.usdToCnyRate)
        }
        return String(format: "%.2f %@", locale: Locale(identifier: "en_US_POSIX"), value, balCurrency)
    }
}

private struct AccountEditorSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var store: AppStore

    let isEditing: Bool
    @State private var kind: AccountModalKind

    // PAYG fields
    @State private var paygAccountId: String
    @State private var paygProviderId: String
    @State private var paygName: String
    @State private var paygApiKey = ""
    @State private var paygManualBalance = ""
    @State private var paygMonthlyBudget = ""
    @State private var paygEnabled: Bool

    // Subscription fields
    @State private var planId: String
    @State private var planProviderId: String
    @State private var planName: String
    @State private var planMonthlyFeeUsd: String
    @State private var planEnabled: Bool
    @State private var selectedTemplateId = "__custom__"
    @State private var windowLabel: String
    @State private var windowHours: String
    @State private var windowUnit: String
    @State private var windowLimit: String
    @State private var additionalWindows: [SubscriptionWindowRuleDTO] = []

    private let paygProviders = [
        "deepseek_harness",
        "kimi_code",
        "minimax_code",
        "qcode",
        "claude_code",
        "codex",
        "workbuddy",
        "cursor",
        "github_copilot",
        "windsurf"
    ]

    init(
        store: AppStore,
        initialKind: AccountModalKind,
        editingPayg: PaygAccountDTO?,
        editingPlan: SubscriptionPlanDTO?
    ) {
        self.store = store
        self.isEditing = (editingPayg != nil || editingPlan != nil)
        _kind = State(initialValue: initialKind)

        let rate = TokenVaneFormatters.exchangeRate(from: store.usdToCnyRate)
        func toInputAmount(_ microUsd: Int64?) -> String {
            guard let microUsd, microUsd > 0 else { return "" }
            let usd = Double(microUsd) / 1_000_000.0
            let display = store.currency.uppercased() == "CNY" ? usd * rate : usd
            return String(format: "%.2f", locale: Locale(identifier: "en_US_POSIX"), display)
        }

        _paygAccountId = State(initialValue: editingPayg?.accountId ?? "payg_\(UUID().uuidString.prefix(8).lowercased())")
        let initialPaygProvider = editingPayg?.providerId ?? "deepseek_harness"
        _paygProviderId = State(initialValue: initialPaygProvider)
        _paygName = State(initialValue: editingPayg?.accountName ?? "\(TokenVaneFormatters.providerDisplayName(initialPaygProvider)) 账户")
        _paygManualBalance = State(initialValue: toInputAmount(editingPayg?.manualBalanceMicroUsd))
        _paygMonthlyBudget = State(initialValue: toInputAmount(editingPayg?.monthlyBudgetMicroUsd))
        _paygEnabled = State(initialValue: editingPayg?.enabled ?? true)

        _planId = State(initialValue: editingPlan?.planId ?? "sub_\(UUID().uuidString.prefix(8).lowercased())")
        _planProviderId = State(initialValue: editingPlan?.providerId ?? "claude_code")
        _planName = State(initialValue: editingPlan?.planName ?? "Claude Pro")
        let feeUsd = Double(editingPlan?.monthlyFeeMicroUsd ?? 20_000_000) / 1_000_000.0
        _planMonthlyFeeUsd = State(initialValue: String(format: "%.2f", locale: Locale(identifier: "en_US_POSIX"), feeUsd))
        _planEnabled = State(initialValue: editingPlan?.enabled ?? true)

        let firstWin = editingPlan?.windows.first ?? SubscriptionWindowRuleDTO(
            windowId: "5h",
            label: "5h 滚动窗口",
            durationHours: 5,
            rolling: true,
            unit: "micro_usd",
            limitValue: 5_000_000
        )
        _windowLabel = State(initialValue: firstWin.label)
        _windowHours = State(initialValue: String(firstWin.durationHours))
        _windowUnit = State(initialValue: firstWin.unit)
        if firstWin.unit == "micro_usd" {
            _windowLimit = State(initialValue: String(format: "%.2f", locale: Locale(identifier: "en_US_POSIX"), Double(firstWin.limitValue) / 1_000_000.0))
        } else {
            _windowLimit = State(initialValue: String(firstWin.limitValue))
        }
        _additionalWindows = State(initialValue: Array((editingPlan?.windows ?? []).dropFirst()))
    }

    var body: some View {
        NavigationStack {
            Form {
                if !isEditing {
                    Picker(L10n.tr("accounts.editor.kind", default: "账户类型"), selection: $kind) {
                        ForEach(AccountModalKind.allCases) { k in
                            Text(k.label).tag(k)
                        }
                    }
                    .pickerStyle(.segmented)
                }

                if kind == .payg {
                    paygFormSections
                } else {
                    subscriptionFormSections
                }
            }
            .navigationTitle(
                isEditing
                    ? L10n.tr("accounts.editor.edit_title", default: "编辑账户")
                    : L10n.tr("accounts.editor.new_title", default: "添加账户")
            )
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.tr("pairing.alert.cancel", default: "取消")) {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.tr("common.save", default: "保存")) {
                        Task { await saveForm() }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var paygFormSections: some View {
        Section(L10n.tr("accounts.payg.section.basic", default: "基础信息")) {
            Picker(L10n.tr("accounts.editor.provider", default: "服务商"), selection: $paygProviderId) {
                ForEach(paygProviders, id: \.self) { pid in
                    Text(TokenVaneFormatters.providerDisplayName(pid)).tag(pid)
                }
            }
            .onChange(of: paygProviderId) { _, newPid in
                if !isEditing {
                    paygName = "\(TokenVaneFormatters.providerDisplayName(newPid)) 账户"
                }
            }

            TextField(L10n.tr("accounts.editor.name", default: "账户备注名称"), text: $paygName)
            Toggle(L10n.tr("accounts.editor.enabled", default: "启用追踪"), isOn: $paygEnabled)
        }

        Section(L10n.tr("accounts.payg.section.auth_budget", default: "密钥与预算")) {
            SecureField(
                isEditing
                    ? L10n.tr("accounts.payg.apikey_edit_placeholder", default: "API Key（留空则保持 Keychain 现有密钥）")
                    : L10n.tr("accounts.payg.apikey_placeholder", default: "API Key（可选，存入 Keychain 用于刷新官方余额）"),
                text: $paygApiKey
            )
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()

            HStack {
                Text(L10n.trFormat("accounts.payg.manual_balance", default: "当前余额 (%@)", store.currency))
                Spacer()
                TextField("0.00", text: $paygManualBalance)
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 110)
            }

            HStack {
                Text(L10n.trFormat("accounts.payg.monthly_budget", default: "月预算预警 (%@)", store.currency))
                Spacer()
                TextField("0.00", text: $paygMonthlyBudget)
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 110)
            }
        }
    }

    @ViewBuilder
    private var subscriptionFormSections: some View {
        if !isEditing && !store.subscriptionTemplates.isEmpty {
            Section(L10n.tr("accounts.sub.section.template", default: "目录套餐预设")) {
                Picker(L10n.tr("accounts.sub.template_picker", default: "选择模板"), selection: $selectedTemplateId) {
                    Text(L10n.tr("accounts.sub.template_custom", default: "自定义方案")).tag("__custom__")
                    ForEach(store.subscriptionTemplates) { tpl in
                        Text("\(tpl.planName) (\(TokenVaneFormatters.providerDisplayName(tpl.providerId)))")
                            .tag(tpl.templateId)
                    }
                }
                .onChange(of: selectedTemplateId) { _, tplId in
                    if let tpl = store.subscriptionTemplates.first(where: { $0.templateId == tplId }) {
                        applyTemplate(tpl)
                    }
                }
            }
        }

        Section(L10n.tr("accounts.sub.section.plan", default: "订阅详情")) {
            TextField(L10n.tr("accounts.editor.provider", default: "服务商 ID"), text: $planProviderId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            TextField(L10n.tr("accounts.sub.plan_name", default: "订阅名称"), text: $planName)
            HStack {
                Text(L10n.tr("accounts.sub.monthly_fee_usd", default: "月费 (USD)"))
                Spacer()
                TextField("20.00", text: $planMonthlyFeeUsd)
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 110)
            }
            Toggle(L10n.tr("accounts.editor.enabled", default: "启用监控"), isOn: $planEnabled)
        }

        Section(L10n.tr("accounts.sub.section.window", default: "主配额周期规则")) {
            TextField(L10n.tr("accounts.sub.window_label", default: "窗口标签"), text: $windowLabel)
            HStack {
                Text(L10n.tr("accounts.sub.window_hours", default: "周期时长 (小时)"))
                Spacer()
                TextField("5", text: $windowHours)
                    .keyboardType(.numberPad)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 90)
            }
            Picker(L10n.tr("accounts.sub.window_unit", default: "计价单位"), selection: $windowUnit) {
                Text("USD").tag("micro_usd")
                Text("Tokens").tag("tokens")
            }
            .pickerStyle(.segmented)

            HStack {
                Text(windowUnit == "micro_usd" ? "上限 (USD)" : "上限 (Tokens)")
                Spacer()
                TextField(windowUnit == "micro_usd" ? "5.00" : "1000000", text: $windowLimit)
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 110)
            }
        }
    }

    private func applyTemplate(_ tpl: SubscriptionPlanTemplateDTO) {
        planProviderId = tpl.providerId
        planName = tpl.planName
        let feeUsd = Double(tpl.monthlyFeeMicroUsd) / 1_000_000.0
        planMonthlyFeeUsd = String(format: "%.2f", locale: Locale(identifier: "en_US_POSIX"), feeUsd)
        if let first = tpl.windows.first {
            windowLabel = first.label
            windowHours = String(first.durationHours)
            windowUnit = first.unit
            if first.unit == "micro_usd" {
                windowLimit = String(format: "%.2f", locale: Locale(identifier: "en_US_POSIX"), Double(first.limitValue) / 1_000_000.0)
            } else {
                windowLimit = String(first.limitValue)
            }
        }
        additionalWindows = Array(tpl.windows.dropFirst())
    }

    private func parseDisplayCurrencyToMicroUsd(_ raw: String) -> Int64? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let val = Double(trimmed), val.isFinite, val >= 0 else {
            return nil
        }
        let usd = store.currency.uppercased() == "CNY"
            ? val / TokenVaneFormatters.exchangeRate(from: store.usdToCnyRate)
            : val
        return Int64((usd * 1_000_000.0).rounded())
    }

    private func saveForm() async {
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        if kind == .payg {
            let trimmedName = paygName.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmedName.isEmpty else { return }
            let manualMicro = parseDisplayCurrencyToMicroUsd(paygManualBalance)
            let budgetMicro = parseDisplayCurrencyToMicroUsd(paygMonthlyBudget)
            let account = PaygAccountDTO(
                accountId: paygAccountId,
                providerId: paygProviderId,
                accountName: trimmedName,
                enabled: paygEnabled,
                manualBalanceMicroUsd: manualMicro,
                balanceAnchorMs: manualMicro != nil ? nowMs : nil,
                monthlyBudgetMicroUsd: budgetMicro,
                isAuto: false,
                updatedAtMs: nowMs
            )
            let trimmedKey = paygApiKey.trimmingCharacters(in: .whitespacesAndNewlines)
            let keyArg: String? = isEditing && trimmedKey.isEmpty ? nil : trimmedKey
            if await store.upsertPaygAccount(account, apiKey: keyArg) {
                dismiss()
            }
        } else {
            let trimmedName = planName.trimmingCharacters(in: .whitespacesAndNewlines)
            let trimmedProvider = planProviderId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmedName.isEmpty, !trimmedProvider.isEmpty else { return }
            let feeMicro = Int64(((Double(planMonthlyFeeUsd) ?? 0) * 1_000_000.0).rounded())
            let hours = max(1, Int64(windowHours) ?? 5)
            let rawLimit = Double(windowLimit) ?? 0
            let limitVal: Int64 = windowUnit == "micro_usd"
                ? Int64((rawLimit * 1_000_000.0).rounded())
                : Int64(rawLimit.rounded())

            let primaryWin = SubscriptionWindowRuleDTO(
                windowId: "\(hours)h",
                label: windowLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "\(hours)h" : windowLabel,
                durationHours: hours,
                rolling: true,
                unit: windowUnit,
                limitValue: max(1, limitVal)
            )
            let plan = SubscriptionPlanDTO(
                planId: planId,
                providerId: trimmedProvider,
                planName: trimmedName,
                enabled: planEnabled,
                monthlyFeeMicroUsd: max(0, feeMicro),
                windows: [primaryWin] + additionalWindows,
                isUserConfigured: true,
                autoSource: nil,
                updatedAtMs: nowMs
            )
            if await store.upsertSubscriptionPlan(plan) {
                dismiss()
            }
        }
    }
}
