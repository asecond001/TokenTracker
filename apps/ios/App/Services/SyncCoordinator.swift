@preconcurrency import BackgroundTasks
import Foundation
@preconcurrency import Network

public enum SyncConnectionState: Equatable, Sendable {
    case offline
    case syncing
    case live
    case error(String)
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
                await self.syncExchangeRateIfAuto()
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
        let sources: [(String, String)] = [
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
                await onDataUpdated?()
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
