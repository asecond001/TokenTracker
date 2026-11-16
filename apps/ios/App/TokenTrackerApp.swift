import SwiftUI

@main
struct TokenTrackerApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @State private var store = AppStore()

    var body: some Scene {
        WindowGroup {
            RootSplitOrTabView(store: store)
                .preferredColorScheme(store.preferredColorScheme)
                .task {
                    store.registerBackgroundTasks()
                    await store.bootstrapAndSeedIfNeeded()
                }
                .onOpenURL { url in
                    Task {
                        store.selectedTab = .settings
                        _ = await store.applyPairingPayload(url.absoluteString, passphrase: nil)
                    }
                }
        }
        .onChange(of: scenePhase) { _, newPhase in
            store.handleScenePhase(newPhase)
        }
    }
}
