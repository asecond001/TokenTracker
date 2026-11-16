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
        }
        .onChange(of: scenePhase) { _, newPhase in
            store.handleScenePhase(newPhase)
        }
    }
}
