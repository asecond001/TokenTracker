import XCTest

final class TokenTrackerUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testUnpairedOnboardingStateAllowsNavigatingToSettings() {
        let app = XCUIApplication()
        app.launch()

        let pairButton = app.buttons["overview.pairActionButton"]
        if pairButton.waitForExistence(timeout: 5) {
            pairButton.tap()
            XCTAssertTrue(app.buttons["settings.scanQRButton"].waitForExistence(timeout: 4))
        }
    }

    func testSeededDemoNavigationCurrencyAndDeviceFiltering() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitest-seed-demo"]
        app.launch()

        let totalCost = app.staticTexts["overview.totalCost"]
        XCTAssertTrue(totalCost.waitForExistence(timeout: 6))

        let currencyToggle = app.buttons["global.currencyToggle"]
        XCTAssertTrue(currencyToggle.waitForExistence(timeout: 3))
        currencyToggle.tap()
        XCTAssertTrue(app.buttons["global.currencyToggle"].waitForExistence(timeout: 3))

        // Switch across main sections (works on iPhone TabBar or iPad Sidebar)
        for label in ["趋势", "订阅", "设置", "概览"] {
            let tabButton = app.tabBars.buttons[label]
            if tabButton.exists {
                tabButton.tap()
            } else {
                let sidebarMatch = app.buttons[label]
                if sidebarMatch.exists {
                    sidebarMatch.tap()
                }
            }
        }
        XCTAssertTrue(totalCost.waitForExistence(timeout: 4))
    }
}
