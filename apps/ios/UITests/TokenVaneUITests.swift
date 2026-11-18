import XCTest

final class TokenVaneUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    private func makeApp(extraArguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = [
            "-AppleLanguages", "(zh-Hans)",
            "-AppleLocale", "zh_Hans_CN"
        ] + extraArguments
        return app
    }

    @MainActor
    func testUnpairedOnboardingStateAllowsNavigatingToSettings() {
        let app = makeApp()
        app.launch()

        let pairButton = app.buttons["overview.pairActionButton"]
        if pairButton.waitForExistence(timeout: 5) {
            pairButton.tap()
            XCTAssertTrue(app.buttons["settings.scanQRButton"].waitForExistence(timeout: 4))
        }
    }

    @MainActor
    func testSeededDemoNavigationCurrencyAndDeviceFiltering() {
        let app = makeApp(extraArguments: ["--uitest-seed-demo"])
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

    @MainActor
    func testPairingConfirmationAlertFlow() {
        let app = makeApp(extraArguments: ["--uitest-seed-demo", "--uitest-stage-pairing"])
        app.launch()

        let byIdentifier = app.buttons["pairing.confirmButton"]
        let confirmButton: XCUIElement
        if byIdentifier.waitForExistence(timeout: 3) {
            confirmButton = byIdentifier
        } else {
            let byTitle = app.alerts.buttons["连接"]
            XCTAssertTrue(byTitle.waitForExistence(timeout: 4))
            confirmButton = byTitle
        }
        confirmButton.tap()

        let settingsTab = app.tabBars.buttons["设置"]
        if settingsTab.exists {
            settingsTab.tap()
        } else {
            let sidebarSettings = app.buttons["设置"]
            if sidebarSettings.exists {
                sidebarSettings.tap()
            }
        }

        let workerField = app.textFields["settings.workerURLField"]
        XCTAssertTrue(workerField.waitForExistence(timeout: 4))
        let predicate = NSPredicate(format: "value == %@", "https://uitest-sync.tokenvane.local")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: workerField)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 5.0), .completed)
    }
}
