import SwiftUI
import WidgetKit

@main
struct TokenTrackerWidgetBundle: WidgetBundle {
    var body: some Widget {
        OverviewHomeWidget()
        QuotaLockScreenWidget()
        CodingLiveActivityWidget()
    }
}
