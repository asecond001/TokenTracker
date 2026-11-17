import BackgroundTasks
import Foundation
import Network

public enum SyncConnectionState: Equatable, Sendable {
    case offline
    case syncing
    case live
    case error(String)
}

@MainActor
public final class SyncCoordinator {
    private let core: CoreClientActor
    private let secrets: SecretStoring
    private let session: URLSession
    private let pathMonitor = NWPathMonitor()
    private let monitorQueue = DispatchQueue(label: "com.asecond.tokenvane.network")

    private var sseTask: Task<Void, Never>?
    private var isForeground = false
    private var reconnectDelayIndex = 0
    private let backoffSeconds: [TimeInterval] = [2, 5, 15, 60]

    public var onStateChanged: ((SyncConnectionState) -> Void)?
    public var onDataUpdated: (() async -> Void)?

    public init(
        core: CoreClientActor,
        secrets: SecretStoring = KeychainStore.shared,
        session: URLSession = .shared
    ) {
        self.core = core
        self.secrets = secrets
        self.session = session
        startNetworkMonitor()
    }

    deinit {
        pathMonitor.cancel()
        sseTask?.cancel()
    }

    public func registerBackgroundTasks() {
        BGTaskScheduler.shared.register(
            forTaskWithIdentifier: AppGroupPaths.syncRefreshTaskIdentifier,
            using: nil
        ) { [weak self] task in
            guard let refreshTask = task as? BGAppRefreshTask else {
                task.setTaskCompleted(success: false)
                return
            }
            self?.handleBackgroundRefresh(refreshTask)
        }
    }

    public func handleScenePhaseChanged(toForeground: Bool) {
        isForeground = toForeground
        if toForeground {
            Task {
                await pullIncremental()
                startSSEStream()
                await syncExchangeRateIfAuto()
            }
        } else {
            stopSSEStream()
            scheduleBackgroundRefresh()
            Task {
                await core.checkpointForSuspend()
            }
        }
    }

    public func startSSEStream() {
        stopSSEStream()
        guard isForeground else { return }

        sseTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled && self.isForeground {
                do {
                    try await self.runOneSSEConnection()
                } catch is CancellationError {
                    break
                } catch {
                    if !self.isForeground { break }
                    self.onStateChanged?(.error(error.localizedDescription))
                    let delay = self.backoffSeconds[min(self.reconnectDelayIndex, self.backoffSeconds.count - 1)]
                    self.reconnectDelayIndex = min(self.reconnectDelayIndex + 1, self.backoffSeconds.count - 1)
                    try? await Task.sleep(for: .seconds(delay))
                }
            }
        }
    }

    public func stopSSEStream() {
        sseTask?.cancel()
        sseTask = nil
    }

    @discardableResult
    public func pullIncremental() async -> Bool {
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

            reconnectDelayIndex = 0
            onStateChanged?(sseTask != nil ? .live : .offline)
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

    private func syncExchangeRateIfAuto() async {
        guard let bootstrap = try? await core.getBootstrap(), bootstrap.preferences.exchangeRateAuto else {
            return
        }
        _ = await syncExchangeRateNow()
    }

    private func runOneSSEConnection() async throws {
        let bootstrap = try await core.getBootstrap()
        guard bootstrap.syncConfig.enabled,
              let rawURL = bootstrap.syncConfig.workerUrl,
              let baseURL = URL(string: rawURL),
              let token = try secrets.loadSecret(account: .syncBearerToken),
              !token.isEmpty else {
            onStateChanged?(.offline)
            try? await Task.sleep(for: .seconds(15))
            return
        }

        let sseURL = baseURL.appendingPathComponent("v1/sync/events")
        var req = URLRequest(url: sseURL)
        req.timeoutInterval = 300
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")

        let (bytes, response) = try await session.bytes(for: req)
        guard let httpResp = response as? HTTPURLResponse, (200..<300).contains(httpResp.statusCode) else {
            let code = (response as? HTTPURLResponse)?.statusCode ?? -1
            throw NSError(domain: "SSE", code: code, userInfo: [NSLocalizedDescriptionKey: "SSE HTTP \(code)"])
        }

        reconnectDelayIndex = 0
        onStateChanged?(.live)

        var parser = SSEFrameParser()
        for try await byte in bytes {
            if Task.isCancelled || !isForeground { break }
            if let frame = parser.feedByte(byte), frame.event == "sync_update" {
                await pullIncremental()
            }
        }
        if let trailing = parser.finish(), trailing.event == "sync_update" {
            await pullIncremental()
        }
    }

    private func startNetworkMonitor() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            guard path.status == .satisfied else { return }
            Task { @MainActor [weak self] in
                guard let self, self.isForeground else { return }
                self.reconnectDelayIndex = 0
                await self.pullIncremental()
                if self.sseTask == nil {
                    self.startSSEStream()
                }
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
        let workTask = Task { [weak self] in
            guard let self else {
                task.setTaskCompleted(success: false)
                return
            }
            let ok = await self.pullIncremental()
            await self.core.checkpointForSuspend()
            task.setTaskCompleted(success: ok)
        }
        task.expirationHandler = {
            workTask.cancel()
        }
    }
}
