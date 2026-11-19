@preconcurrency import BackgroundTasks
import Foundation
@preconcurrency import Network

public enum SyncConnectionState: Equatable, Sendable {
    case offline
    case syncing
    case live
    case error(String)
}

public struct CatalogRefreshSummary: Equatable, Sendable {
    public let rateUpdated: Double?
    public let pricingCount: UInt64?
    public let subscriptionCount: UInt64?

    public init(rateUpdated: Double? = nil, pricingCount: UInt64? = nil, subscriptionCount: UInt64? = nil) {
        self.rateUpdated = rateUpdated
        self.pricingCount = pricingCount
        self.subscriptionCount = subscriptionCount
    }
}

public enum CloudSyncProvisionStep: String, CaseIterable, Sendable {
    case preparing
    case resolvingAccount = "resolving_account"
    case deploying
    case enablingSubdomain = "enabling_subdomain"
    case initialSync = "initial_sync"

    public var label: String {
        switch self {
        case .preparing: return L10n.tr("provision.step.preparing", default: "生成配置与脚本包")
        case .resolvingAccount: return L10n.tr("provision.step.resolving_account", default: "解析 Cloudflare 账户")
        case .deploying: return L10n.tr("provision.step.deploying", default: "上传 Worker 与存储引擎")
        case .enablingSubdomain: return L10n.tr("provision.step.enabling_subdomain", default: "启用 workers.dev 访问域")
        case .initialSync: return L10n.tr("provision.step.initial_sync", default: "写入凭据并测试同步")
        }
    }
}

public struct CloudSyncProvisionOutcome: Equatable, Sendable {
    public let workerName: String
    public let workerURL: String
    public let bearerToken: String
    public let e2eeEnabled: Bool
    public let e2eePassphrase: String?
    public let pairingURI: String
    public let initialSyncOK: Bool

    public init(
        workerName: String,
        workerURL: String,
        bearerToken: String,
        e2eeEnabled: Bool,
        e2eePassphrase: String?,
        pairingURI: String,
        initialSyncOK: Bool
    ) {
        self.workerName = workerName
        self.workerURL = workerURL
        self.bearerToken = bearerToken
        self.e2eeEnabled = e2eeEnabled
        self.e2eePassphrase = e2eePassphrase
        self.pairingURI = pairingURI
        self.initialSyncOK = initialSyncOK
    }
}

private struct UncheckedSendableBox<T>: @unchecked Sendable {
    let value: T
}

private final class LockedTaskBag: @unchecked Sendable {
    private let lock = NSLock()
    private var pollTask: Task<Void, Never>?
    private var pullTask: Task<Bool, Never>?

    func setPollTask(_ task: Task<Void, Never>?) {
        lock.lock()
        let prev = pollTask
        pollTask = task
        lock.unlock()
        prev?.cancel()
    }

    func getPullTask() -> Task<Bool, Never>? {
        lock.lock()
        defer { lock.unlock() }
        return pullTask
    }

    func setPullTask(_ task: Task<Bool, Never>?) {
        lock.lock()
        pullTask = task
        lock.unlock()
    }

    func cancelAll() {
        lock.lock()
        let poll = pollTask
        let pull = pullTask
        pollTask = nil
        pullTask = nil
        lock.unlock()
        poll?.cancel()
        pull?.cancel()
    }
}

@MainActor
public final class SyncCoordinator {
    public static let exchangeRateThrottleMs: Int64 = 12 * 3_600_000
    public static let balanceCooldownMs: Int64 = 300_000
    private static let cloudflareApiBase = "https://api.cloudflare.com/client/v4"

    private let core: CoreClientActor
    private let secrets: SecretStoring
    private let session: URLSession
    private let pollInterval: Duration
    private let pathMonitor = NWPathMonitor()
    private let monitorQueue = DispatchQueue(label: "com.asecond.tokenvane.network")
    private let taskBag = LockedTaskBag()

    private var pullRequestedWhileRunning = false
    private static var didRegisterBackgroundTasks = false
    private var isForeground = false
    private var lastBalanceFetchAtMs: [String: Int64] = [:]

    public var onStateChanged: ((SyncConnectionState) -> Void)?
    public var onDataUpdated: (() async -> Void)?
    public var onBackgroundSyncCompleted: (() async -> Void)?

    public init(
        core: CoreClientActor,
        secrets: SecretStoring = KeychainStore.shared,
        session: URLSession = .shared,
        pollInterval: Duration = .seconds(30)
    ) {
        self.core = core
        self.secrets = secrets
        self.session = session
        self.pollInterval = pollInterval
        startNetworkMonitor()
    }

    deinit {
        pathMonitor.cancel()
        taskBag.cancelAll()
    }

    /// Must be called before the application finishes launching (see `TokenVaneApp.init`).
    /// Registering twice raises an exception inside BGTaskScheduler, so the call is idempotent.
    public func registerBackgroundTasks() {
        guard !Self.didRegisterBackgroundTasks else { return }
        Self.didRegisterBackgroundTasks = true
        BGTaskScheduler.shared.register(
            forTaskWithIdentifier: AppGroupPaths.syncRefreshTaskIdentifier,
            using: nil
        ) { [weak self] task in
            guard let refreshTask = task as? BGAppRefreshTask else {
                task.setTaskCompleted(success: false)
                return
            }
            let box = UncheckedSendableBox(value: refreshTask)
            Task { @MainActor [weak self] in
                guard let self else {
                    box.value.setTaskCompleted(success: false)
                    return
                }
                self.handleBackgroundRefresh(box.value)
            }
        }
    }

    public func handleScenePhaseChanged(toForeground: Bool) {
        isForeground = toForeground
        if toForeground {
            Task { [weak self] in
                guard let self else { return }
                await self.pullIncremental()
                self.startForegroundPolling()
                _ = await self.refreshPublicCatalogs(force: false)
            }
        } else {
            stopForegroundPolling()
            scheduleBackgroundRefresh()
            let coreRef = core
            Task {
                await coreRef.checkpointForSuspend()
            }
        }
    }

    public func startForegroundPolling() {
        stopForegroundPolling()
        guard isForeground else { return }

        let interval = pollInterval
        let task = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: interval)
                guard !Task.isCancelled, let self, self.isForeground else { break }
                _ = await self.pullIncremental()
            }
        }
        taskBag.setPollTask(task)
    }

    public func stopForegroundPolling() {
        taskBag.setPollTask(nil)
    }

    /// Single-flight incremental pull. Concurrent callers (foreground, polling, network recovery,
    /// pull-to-refresh) share the running pull; a request that arrives mid-flight schedules exactly
    /// one follow-up pass so the newest server state is never missed.
    @discardableResult
    public func pullIncremental() async -> Bool {
        if let running = taskBag.getPullTask() {
            pullRequestedWhileRunning = true
            return await running.value
        }
        let task = Task { [weak self] () -> Bool in
            guard let self else { return false }
            var ok = false
            repeat {
                self.pullRequestedWhileRunning = false
                ok = await self.performPull()
            } while self.pullRequestedWhileRunning && !Task.isCancelled
            self.taskBag.setPullTask(nil)
            return ok
        }
        taskBag.setPullTask(task)
        return await task.value
    }

    private func performPull() async -> Bool {
        do {
            let bootstrap = try await core.getBootstrap()
            guard bootstrap.syncConfig.enabled,
                  let rawURL = bootstrap.syncConfig.workerUrl,
                  let baseURL = URL(string: rawURL),
                  let token = try secrets.loadSecret(account: .syncBearerToken),
                  !token.isEmpty else {
                onStateChanged?(.offline)
                return false
            }

            onStateChanged?(.syncing)
            let passphrase = try secrets.loadSecret(account: .e2eePassphrase)
            var sinceMs = bootstrap.syncConfig.lastPullWatermarkMs
            let viewerId = bootstrap.viewerDeviceId
            var hasMore = true
            var iterations = 0

            while hasMore && iterations < 50 {
                iterations += 1
                var components = URLComponents(url: baseURL.appendingPathComponent("v1/sync/pull"), resolvingAgainstBaseURL: false)
                components?.queryItems = [
                    URLQueryItem(name: "since", value: String(sinceMs)),
                    URLQueryItem(name: "exclude_device", value: viewerId),
                    URLQueryItem(name: "limit", value: "200")
                ]
                guard let url = components?.url else {
                    throw URLError(.badURL)
                }

                var req = URLRequest(url: url)
                req.httpMethod = "GET"
                req.timeoutInterval = 20
                req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")

                let (data, response) = try await session.data(for: req)
                guard let httpResp = response as? HTTPURLResponse, (200..<300).contains(httpResp.statusCode) else {
                    let statusCode = (response as? HTTPURLResponse)?.statusCode ?? -1
                    throw NSError(domain: "SyncCoordinator", code: statusCode, userInfo: [NSLocalizedDescriptionKey: "Worker HTTP \(statusCode)"])
                }
                guard let bodyText = String(data: data, encoding: .utf8) else {
                    throw URLError(.cannotDecodeContentData)
                }
                let outcome = try await core.ingestPullResponse(bodyJSON: bodyText, e2eePassphrase: passphrase)
                hasMore = outcome.hasMore
                sinceMs = outcome.nextSinceMs
            }

            onStateChanged?(isForeground ? .live : .offline)
            await onDataUpdated?()
            return true
        } catch {
            onStateChanged?(.error(error.localizedDescription))
            return false
        }
    }

    public func probeHealth(workerURL: String, bearerToken: String?) async -> Result<String, Error> {
        do {
            guard let base = URL(string: workerURL.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                throw URLError(.badURL)
            }
            var req = URLRequest(url: base.appendingPathComponent("healthz"))
            req.timeoutInterval = 10
            if let bearerToken, !bearerToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                req.setValue("Bearer \(bearerToken.trimmingCharacters(in: .whitespacesAndNewlines))", forHTTPHeaderField: "Authorization")
            }
            let (data, response) = try await session.data(for: req)
            guard let httpResp = response as? HTTPURLResponse, (200..<300).contains(httpResp.statusCode) else {
                let code = (response as? HTTPURLResponse)?.statusCode ?? -1
                return .failure(NSError(domain: "SyncCoordinator", code: code, userInfo: [NSLocalizedDescriptionKey: "HTTP \(code)"]))
            }
            let text = String(data: data, encoding: .utf8) ?? "ok"
            return .success(text)
        } catch {
            return .failure(error)
        }
    }

    public func syncExchangeRateNow() async -> Double? {
        if let updated = await fetchAndApplyExchangeRate(nowMs: nil) {
            await onDataUpdated?()
            return updated
        }
        return nil
    }

    private func fetchAndApplyExchangeRate(nowMs: Int64?) async -> Double? {
        let sources: [(String, String)] = [
            ("frankfurter", "https://api.frankfurter.dev/v1/latest?base=USD&symbols=CNY"),
            ("frankfurter", "https://api.frankfurter.app/latest?from=USD&to=CNY"),
            ("open_er", "https://open.er-api.com/v6/latest/USD")
        ]
        for (source, urlString) in sources {
            guard let url = URL(string: urlString) else { continue }
            var req = URLRequest(url: url)
            req.timeoutInterval = 10
            guard let (data, response) = try? await session.data(for: req),
                  let http = response as? HTTPURLResponse,
                  (200..<300).contains(http.statusCode),
                  let rawJSON = String(data: data, encoding: .utf8) else {
                continue
            }
            if let updated = try? await core.applyExchangeRatePayload(source: source, bodyJSON: rawJSON) {
                return updated
            }
        }
        return nil
    }

    @discardableResult
    public func syncExchangeRateIfAuto(nowMs: Int64? = nil) async -> Bool {
        guard let bootstrap = try? await core.getBootstrap(),
              bootstrap.preferences.exchangeRateAuto else {
            return false
        }
        let now = nowMs ?? Int64(Date().timeIntervalSince1970 * 1000)
        if let lastSync = bootstrap.preferences.lastRateSyncAtMs,
           lastSync > 0,
           (now - lastSync) < Self.exchangeRateThrottleMs {
            return false
        }
        return await syncExchangeRateNow() != nil
    }

    @discardableResult
    public func refreshPublicCatalogs(
        force: Bool = false,
        nowMs: Int64? = nil
    ) async -> CatalogRefreshSummary {
        let now = nowMs ?? Int64(Date().timeIntervalSince1970 * 1000)
        let bootstrap = try? await core.getBootstrap()

        func needsSync(_ lastMs: Int64?) -> Bool {
            if force { return true }
            guard let lastMs, lastMs > 0 else { return true }
            return (now - lastMs) >= Self.exchangeRateThrottleMs
        }

        var updatedRate: Double?
        if force || ((bootstrap?.preferences.exchangeRateAuto ?? false) && needsSync(bootstrap?.preferences.lastRateSyncAtMs)) {
            updatedRate = await fetchAndApplyExchangeRate(nowMs: now)
        }

        var updatedPricing: UInt64?
        if force || ((bootstrap?.preferences.autoSyncPricing ?? false) && needsSync(bootstrap?.preferences.lastPricingSyncAtMs)) {
            let effectiveDate = TokenVaneFormatters.formatDayKey(Date(timeIntervalSince1970: Double(now) / 1000.0))
            let pricingSources: [(String, String)] = [
                ("models_dev", "https://models.dev/api.json"),
                ("litellm", "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json")
            ]
            for (source, urlString) in pricingSources {
                guard let url = URL(string: urlString) else { continue }
                var req = URLRequest(url: url)
                req.timeoutInterval = 20
                guard let (data, response) = try? await session.data(for: req),
                      let http = response as? HTTPURLResponse,
                      (200..<300).contains(http.statusCode),
                      let bodyText = String(data: data, encoding: .utf8),
                      let count = try? await core.applyPricingCatalogPayload(
                          source: source,
                          bodyJSON: bodyText,
                          effectiveDate: effectiveDate,
                          nowMs: now
                      ),
                      count > 0 else {
                    continue
                }
                updatedPricing = count
                break
            }
        }

        var updatedSubscriptions: UInt64?
        if force || ((bootstrap?.preferences.autoSyncPricing ?? false) && needsSync(bootstrap?.preferences.lastSubscriptionSyncAtMs)) {
            let subURLs = [
                "https://asecond001.github.io/ai-subscription-catalog/v1/plans.json",
                "https://cdn.jsdelivr.net/gh/asecond001/ai-subscription-catalog@main/catalog/subscription-plans.json",
                "https://raw.githubusercontent.com/asecond001/ai-subscription-catalog/main/catalog/subscription-plans.json"
            ]
            for urlString in subURLs {
                guard let url = URL(string: urlString) else { continue }
                var req = URLRequest(url: url)
                req.timeoutInterval = 15
                guard let (data, response) = try? await session.data(for: req),
                      let http = response as? HTTPURLResponse,
                      (200..<300).contains(http.statusCode),
                      let bodyText = String(data: data, encoding: .utf8),
                      let count = try? await core.applySubscriptionCatalogPayload(
                          bodyJSON: bodyText,
                          nowMs: now
                      ),
                      count > 0 else {
                    continue
                }
                updatedSubscriptions = count
                break
            }
        }

        if updatedRate != nil || updatedPricing != nil || updatedSubscriptions != nil {
            await onDataUpdated?()
        }
        return CatalogRefreshSummary(
            rateUpdated: updatedRate,
            pricingCount: updatedPricing,
            subscriptionCount: updatedSubscriptions
        )
    }

    public static func balanceEndpoints(for providerId: String) -> [String] {
        switch providerId {
        case "kimi_code":
            return [
                "https://api.kimi.com/coding/v1/usages",
                "https://api.moonshot.cn/v1/users/me/balance",
                "https://api.moonshot.ai/v1/users/me/balance"
            ]
        case "deepseek_harness":
            return ["https://api.deepseek.com/user/balance"]
        case "qcode":
            return ["https://api.qcode.cc/api/v1/usage"]
        case "minimax_code":
            return ["https://www.minimaxi.com/v1/token_plan/remains"]
        default:
            return []
        }
    }

    public func refreshOfficialBalance(
        account: PaygAccountDTO,
        bypassCooldown: Bool = false,
        nowMs: Int64? = nil
    ) async throws -> OfficialBalanceApplyResultDTO {
        let now = nowMs ?? Int64(Date().timeIntervalSince1970 * 1000)
        if !bypassCooldown, let lastFetch = lastBalanceFetchAtMs[account.accountId] {
            let elapsed = now - lastFetch
            if elapsed >= 0 && elapsed < Self.balanceCooldownMs {
                let remainingSecs = Int64(ceil(Double(Self.balanceCooldownMs - elapsed) / 1000.0))
                throw NSError(
                    domain: "SyncCoordinator",
                    code: 429,
                    userInfo: [
                        NSLocalizedDescriptionKey: L10n.trFormat(
                            "payg.error.cooldown",
                            default: "请求过于频繁，请 %lld 秒后再试",
                            remainingSecs
                        )
                    ]
                )
            }
        }

        let candidates = Self.balanceEndpoints(for: account.providerId)
        guard !candidates.isEmpty else {
            throw NSError(
                domain: "SyncCoordinator",
                code: 400,
                userInfo: [
                    NSLocalizedDescriptionKey: L10n.tr(
                        "payg.error.unsupported_provider",
                        default: "该服务商暂不支持官方余额接口，请使用手动余额"
                    )
                ]
            )
        }

        let accountKey = try secrets.loadSecret(account: .providerApiKey(account.accountId))
        let providerKey = try secrets.loadSecret(account: .providerApiKey(account.providerId))
        guard let apiKey = [accountKey, providerKey]
            .compactMap({ $0?.trimmingCharacters(in: .whitespacesAndNewlines) })
            .first(where: { !$0.isEmpty }) else {
            throw NSError(
                domain: "SyncCoordinator",
                code: 401,
                userInfo: [
                    NSLocalizedDescriptionKey: L10n.tr(
                        "payg.error.missing_key",
                        default: "请先在账户设置中配置 API Key"
                    )
                ]
            )
        }

        var lastError: Error?
        for endpoint in candidates {
            guard let url = URL(string: endpoint) else { continue }
            var req = URLRequest(url: url)
            req.httpMethod = "GET"
            req.timeoutInterval = 15
            req.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
            req.setValue("application/json", forHTTPHeaderField: "Accept")

            do {
                let (data, response) = try await session.data(for: req)
                guard let http = response as? HTTPURLResponse else {
                    throw URLError(.badServerResponse)
                }
                let bodyText = String(data: data, encoding: .utf8) ?? ""
                guard (200..<300).contains(http.statusCode) else {
                    throw NSError(
                        domain: "SyncCoordinator",
                        code: http.statusCode,
                        userInfo: [
                            NSLocalizedDescriptionKey: "HTTP \(http.statusCode): \(bodyText.prefix(160))"
                        ]
                    )
                }
                let result = try await core.applyOfficialBalancePayload(
                    providerId: account.providerId,
                    endpoint: endpoint,
                    bodyJSON: bodyText,
                    nowMs: now
                )
                lastBalanceFetchAtMs[account.accountId] = now
                await onDataUpdated?()
                return result
            } catch {
                lastError = error
            }
        }

        throw lastError ?? NSError(
            domain: "SyncCoordinator",
            code: -1,
            userInfo: [
                NSLocalizedDescriptionKey: L10n.tr("payg.error.refresh_failed", default: "刷新余额失败")
            ]
        )
    }

    public func provisionCloudflareWorker(
        workerName: String?,
        cfApiToken: String?,
        cfAccountId: String?,
        bearerToken: String?,
        e2eeEnabled: Bool,
        e2eePassphrase: String?,
        onProgress: (@MainActor (CloudSyncProvisionStep) -> Void)? = nil
    ) async throws -> CloudSyncProvisionOutcome {
        onProgress?(.preparing)

        let explicitCFToken = cfApiToken?.trimmingCharacters(in: .whitespacesAndNewlines)
        let storedCFToken = try secrets.loadSecret(account: .cloudflareApiToken)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedCFToken: String
        if let explicitCFToken, !explicitCFToken.isEmpty {
            try secrets.saveSecret(explicitCFToken, account: .cloudflareApiToken)
            resolvedCFToken = explicitCFToken
        } else if let storedCFToken, !storedCFToken.isEmpty {
            resolvedCFToken = storedCFToken
        } else {
            throw NSError(
                domain: "SyncCoordinator",
                code: 400,
                userInfo: [
                    NSLocalizedDescriptionKey: L10n.tr(
                        "provision.error.missing_cf_token",
                        default: "请填写具备 Workers 编辑权限的 Cloudflare API 令牌"
                    )
                ]
            )
        }

        let explicitBearer = bearerToken?.trimmingCharacters(in: .whitespacesAndNewlines)
        let storedBearer = try secrets.loadSecret(account: .syncBearerToken)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let seedBearer = (explicitBearer?.isEmpty == false ? explicitBearer : nil)
            ?? (storedBearer?.isEmpty == false ? storedBearer : nil)

        let explicitPassphrase = e2eePassphrase?.trimmingCharacters(in: .whitespacesAndNewlines)
        let storedPassphrase = try secrets.loadSecret(account: .e2eePassphrase)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let seedPassphrase: String? = if e2eeEnabled {
            (explicitPassphrase?.isEmpty == false ? explicitPassphrase : nil)
                ?? (storedPassphrase?.isEmpty == false ? storedPassphrase : nil)
        } else {
            nil
        }

        let initialBundle = try await core.buildWorkerUploadBundle(
            workerName: workerName,
            bearerToken: seedBearer,
            e2eeEnabled: e2eeEnabled,
            e2eePassphrase: seedPassphrase,
            includeMigration: true
        )

        onProgress?(.resolvingAccount)

        let accountId: String
        let trimmedExplicitAccount = cfAccountId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !trimmedExplicitAccount.isEmpty {
            let validChars = trimmedExplicitAccount.unicodeScalars.allSatisfy {
                CharacterSet.alphanumerics.contains($0) || $0 == "_" || $0 == "-"
            }
            guard validChars else {
                throw NSError(
                    domain: "SyncCoordinator",
                    code: 400,
                    userInfo: [
                        NSLocalizedDescriptionKey: L10n.tr(
                            "provision.error.invalid_account_id",
                            default: "Cloudflare Account ID 仅支持字母、数字、下划线与连字符"
                        )
                    ]
                )
            }
            accountId = trimmedExplicitAccount
        } else {
            let (_, accJSON) = try await performCloudflareJSON(
                method: "GET",
                urlString: "\(Self.cloudflareApiBase)/accounts?per_page=5",
                token: resolvedCFToken
            )
            let accounts = accJSON["result"] as? [[String: Any]]
            guard let id = (accounts?.first?["id"] as? String)?
                .trimmingCharacters(in: .whitespacesAndNewlines),
                  !id.isEmpty else {
                throw NSError(
                    domain: "SyncCoordinator",
                    code: 400,
                    userInfo: [
                        NSLocalizedDescriptionKey: L10n.tr(
                            "provision.error.account_resolve_failed",
                            default: "请确认 Cloudflare API 令牌具备 Account:Read 与 Workers Scripts:Edit 权限，或手动填入 Account ID"
                        )
                    ]
                )
            }
            accountId = id
        }

        let subdomainURL = "\(Self.cloudflareApiBase)/accounts/\(accountId)/workers/subdomain"
        let (subStatus, subData, _) = try await performCloudflareRaw(
            method: "GET",
            urlString: subdomainURL,
            token: resolvedCFToken
        )

        let subdomain: String
        if (200..<300).contains(subStatus),
           let json = try parseCloudflareSuccessJSON(status: subStatus, data: subData),
           let result = json["result"] as? [String: Any],
           let existing = (result["subdomain"] as? String)?
               .trimmingCharacters(in: .whitespacesAndNewlines),
           !existing.isEmpty {
            subdomain = existing
        } else if subStatus == 404 {
            let suffix = String(
                accountId.lowercased().filter { $0.isASCII && ($0.isLetter || $0.isNumber) }.prefix(8)
            )
            let candidate = "tokenvane-\(suffix)"
            let bodyData = try JSONSerialization.data(withJSONObject: ["subdomain": candidate])
            let (putStatus, putData, _) = try await performCloudflareRaw(
                method: "PUT",
                urlString: subdomainURL,
                token: resolvedCFToken,
                contentType: "application/json",
                body: bodyData
            )
            guard let putJSON = try? parseCloudflareSuccessJSON(status: putStatus, data: putData) else {
                throw NSError(
                    domain: "SyncCoordinator",
                    code: putStatus,
                    userInfo: [
                        NSLocalizedDescriptionKey: L10n.tr(
                            "provision.error.subdomain_required",
                            default: "请先在 Cloudflare 控制台 Workers & Pages 中激活一次默认 workers.dev 子域名后重试"
                        )
                    ]
                )
            }
            let result = putJSON["result"] as? [String: Any]
            let resolved = (result?["subdomain"] as? String)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            subdomain = (resolved?.isEmpty == false ? resolved! : candidate)
        } else {
            let msg = parseCloudflareErrorText(status: subStatus, data: subData)
            throw NSError(
                domain: "SyncCoordinator",
                code: subStatus,
                userInfo: [NSLocalizedDescriptionKey: msg]
            )
        }

        let settingsURL = "\(Self.cloudflareApiBase)/accounts/\(accountId)/workers/scripts/\(initialBundle.workerName)/settings"
        let (settingsStatus, settingsData, _) = try await performCloudflareRaw(
            method: "GET",
            urlString: settingsURL,
            token: resolvedCFToken
        )
        let alreadyMigrated: Bool = {
            guard settingsStatus == 200,
                  let obj = try? JSONSerialization.jsonObject(with: settingsData) as? [String: Any],
                  let res = obj["result"] as? [String: Any],
                  let tag = res["migration_tag"] as? String else {
                return false
            }
            return !tag.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }()

        onProgress?(.deploying)
        let scriptURL = "\(Self.cloudflareApiBase)/accounts/\(accountId)/workers/scripts/\(initialBundle.workerName)"
        var uploadBundle = initialBundle
        if alreadyMigrated {
            uploadBundle = try await core.buildWorkerUploadBundle(
                workerName: initialBundle.workerName,
                bearerToken: initialBundle.bearerToken,
                e2eeEnabled: initialBundle.e2eeEnabled,
                e2eePassphrase: initialBundle.e2eePassphrase,
                includeMigration: false
            )
        }

        guard let uploadData = Data(base64Encoded: uploadBundle.multipartBodyBase64) else {
            throw NSError(
                domain: "SyncCoordinator",
                code: 500,
                userInfo: [NSLocalizedDescriptionKey: "Invalid worker bundle payload"]
            )
        }

        var (putStatus, putData, putText) = try await performCloudflareRaw(
            method: "PUT",
            urlString: scriptURL,
            token: resolvedCFToken,
            contentType: uploadBundle.contentType,
            body: uploadData
        )

        if !alreadyMigrated && Self.isMigrationPreconditionError(status: putStatus, bodyText: putText) {
            let retryBundle = try await core.buildWorkerUploadBundle(
                workerName: initialBundle.workerName,
                bearerToken: initialBundle.bearerToken,
                e2eeEnabled: initialBundle.e2eeEnabled,
                e2eePassphrase: initialBundle.e2eePassphrase,
                includeMigration: false
            )
            if let retryData = Data(base64Encoded: retryBundle.multipartBodyBase64) {
                (putStatus, putData, putText) = try await performCloudflareRaw(
                    method: "PUT",
                    urlString: scriptURL,
                    token: resolvedCFToken,
                    contentType: retryBundle.contentType,
                    body: retryData
                )
            }
        }
        _ = try parseCloudflareSuccessJSON(status: putStatus, data: putData)

        onProgress?(.enablingSubdomain)
        let enableURL = "\(Self.cloudflareApiBase)/accounts/\(accountId)/workers/scripts/\(initialBundle.workerName)/subdomain"
        let enableBody = try JSONSerialization.data(withJSONObject: ["enabled": true])
        _ = try await performCloudflareJSON(
            method: "POST",
            urlString: enableURL,
            token: resolvedCFToken,
            contentType: "application/json",
            body: enableBody
        )

        let workerURL = "https://\(initialBundle.workerName).\(subdomain).workers.dev"

        onProgress?(.initialSync)
        try secrets.saveSecret(initialBundle.bearerToken, account: .syncBearerToken)
        if initialBundle.e2eeEnabled, let pass = initialBundle.e2eePassphrase, !pass.isEmpty {
            try secrets.saveSecret(pass, account: .e2eePassphrase)
        } else if !initialBundle.e2eeEnabled {
            try secrets.deleteSecret(account: .e2eePassphrase)
        }

        _ = try await core.updateSyncConfig(
            enabled: true,
            workerURL: workerURL,
            e2eeEnabled: initialBundle.e2eeEnabled
        )
        let initialSyncOK = await pullIncremental()
        let pairingURI = DesktopPairingPayload.makePairingURI(
            url: workerURL,
            token: initialBundle.bearerToken,
            e2ee: initialBundle.e2eeEnabled,
            key: initialBundle.e2eePassphrase
        ) ?? ""

        return CloudSyncProvisionOutcome(
            workerName: initialBundle.workerName,
            workerURL: workerURL,
            bearerToken: initialBundle.bearerToken,
            e2eeEnabled: initialBundle.e2eeEnabled,
            e2eePassphrase: initialBundle.e2eePassphrase,
            pairingURI: pairingURI,
            initialSyncOK: initialSyncOK
        )
    }

    private static func isMigrationPreconditionError(status: Int, bodyText: String) -> Bool {
        guard !(200..<300).contains(status) else { return false }
        let lower = bodyText.lowercased()
        return bodyText.contains("10079") || lower.contains("migration") || lower.contains("already")
    }

    private func performCloudflareRaw(
        method: String,
        urlString: String,
        token: String,
        contentType: String? = nil,
        body: Data? = nil
    ) async throws -> (status: Int, data: Data, text: String) {
        guard let url = URL(string: urlString) else {
            throw URLError(.badURL)
        }
        var req = URLRequest(url: url)
        req.httpMethod = method
        req.timeoutInterval = 30
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        if let contentType {
            req.setValue(contentType, forHTTPHeaderField: "Content-Type")
        }
        req.httpBody = body

        let (data, response) = try await session.data(for: req)
        let status = (response as? HTTPURLResponse)?.statusCode ?? -1
        let text = String(data: data, encoding: .utf8) ?? ""
        return (status, data, text)
    }

    private func performCloudflareJSON(
        method: String,
        urlString: String,
        token: String,
        contentType: String? = nil,
        body: Data? = nil
    ) async throws -> (Int, [String: Any]) {
        let (status, data, _) = try await performCloudflareRaw(
            method: method,
            urlString: urlString,
            token: token,
            contentType: contentType,
            body: body
        )
        guard let json = try parseCloudflareSuccessJSON(status: status, data: data) else {
            throw NSError(
                domain: "SyncCoordinator",
                code: status,
                userInfo: [NSLocalizedDescriptionKey: parseCloudflareErrorText(status: status, data: data)]
            )
        }
        return (status, json)
    }

    private func parseCloudflareSuccessJSON(status: Int, data: Data) throws -> [String: Any]? {
        guard (200..<300).contains(status) else {
            throw NSError(
                domain: "SyncCoordinator",
                code: status,
                userInfo: [NSLocalizedDescriptionKey: parseCloudflareErrorText(status: status, data: data)]
            )
        }
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        if let success = json["success"] as? Bool, !success {
            throw NSError(
                domain: "SyncCoordinator",
                code: status,
                userInfo: [NSLocalizedDescriptionKey: parseCloudflareErrorText(status: status, data: data)]
            )
        }
        return json
    }

    private func parseCloudflareErrorText(status: Int, data: Data) -> String {
        if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let errors = json["errors"] as? [[String: Any]],
           let first = errors.first {
            let msg = (first["message"] as? String) ?? "Cloudflare API 请求异常"
            if let code = first["code"] as? Int {
                return "HTTP \(status): [\(code)] \(msg)"
            }
            return "HTTP \(status): \(msg)"
        }
        let raw = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if raw.isEmpty {
            return "HTTP \(status)"
        }
        return "HTTP \(status): \(String(raw.prefix(200)))"
    }

    private func startNetworkMonitor() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            guard path.status == .satisfied else { return }
            Task { @MainActor [weak self] in
                guard let self, self.isForeground else { return }
                await self.pullIncremental()
            }
        }
        pathMonitor.start(queue: monitorQueue)
    }

    private func scheduleBackgroundRefresh() {
        let request = BGAppRefreshTaskRequest(identifier: AppGroupPaths.syncRefreshTaskIdentifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    private func handleBackgroundRefresh(_ task: BGAppRefreshTask) {
        scheduleBackgroundRefresh()
        let box = UncheckedSendableBox(value: task)
        let workTask = Task { @MainActor [weak self] in
            guard let self else {
                box.value.setTaskCompleted(success: false)
                return
            }
            let ok = await self.pullIncremental()
            if !ok {
                await self.onBackgroundSyncCompleted?()
            }
            await self.core.checkpointForSuspend()
            box.value.setTaskCompleted(success: ok)
        }
        task.expirationHandler = {
            workTask.cancel()
        }
    }
}
