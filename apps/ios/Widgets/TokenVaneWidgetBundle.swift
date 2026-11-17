import SwiftUI
import WidgetKit

@main
struct TokenVaneWidgetBundle: WidgetBundle {
    var body: some Widget {
        OverviewHomeWidget()
        QuotaLockScreenWidget()
        CodingLiveActivityWidget()
    }
}
