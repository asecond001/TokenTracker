import SwiftUI

@main
struct TokenVaneApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @State private var store: AppStore

    init() {
        let store = AppStore()
        // BGTaskScheduler requires every launch handler to be registered before
        // the application finishes launching; App.init runs early enough, `.task` does not.
        store.registerBackgroundTasks()
        WidgetRefreshBridge.shared.register { [weak store] in
            guard let store else { return false }
            return await store.performBackgroundSyncRefresh()
        }
        _store = State(initialValue: store)
    }

    var body: some Scene {
        WindowGroup {
            RootSplitOrTabView(store: store)
                .preferredColorScheme(store.preferredColorScheme)
                .task {
                    await store.bootstrapAndSeedIfNeeded()
                }
                .onOpenURL { url in
                    store.selectedTab = .settings
                    store.stagePairing(raw: url.absoluteString)
                }
        }
        .onChange(of: scenePhase) { _, newPhase in
            store.handleScenePhase(newPhase)
        }
    }
}
