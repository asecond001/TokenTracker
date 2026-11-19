import SwiftUI

public struct TimeRangePickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var store: AppStore

    @State private var mode = 0 // 0 = date range, 1 = precise window
    @State private var startDate: Date
    @State private var endDate: Date
    @State private var preciseStart: Date
    @State private var preciseEnd: Date
    @State private var followNow: Bool

    private let quickWindows: [(label: String, seconds: TimeInterval)] = [
        ("15m", 15 * 60),
        ("1h", 3600),
        ("3h", 3 * 3600),
        ("6h", 6 * 3600),
        ("12h", 12 * 3600),
        ("24h", 24 * 3600),
        ("48h", 48 * 3600)
    ]

    public init(store: AppStore) {
        self.store = store
        let now = Date()
        let cal = Calendar.current
        let today = cal.startOfDay(for: now)
        let initialStart = store.timeRange == .custom
            ? store.customStartDate
            : (cal.date(byAdding: .day, value: -6, to: today) ?? today)
        let initialEnd = store.timeRange == .custom ? store.customEndDate : today
        _startDate = State(initialValue: initialStart)
        _endDate = State(initialValue: initialEnd)

        let pStart = store.customStartMs.map { Date(timeIntervalSince1970: Double($0) / 1000.0) }
            ?? now.addingTimeInterval(-6 * 3600)
        let pEnd = store.customEndMs.map { Date(timeIntervalSince1970: Double($0) / 1000.0) }
            ?? now
        _preciseStart = State(initialValue: pStart)
        _preciseEnd = State(initialValue: pEnd)
        _followNow = State(initialValue: store.customEndMs == nil)
        _mode = State(initialValue: store.customPrecise ? 1 : 0)
    }

    public var body: some View {
        NavigationStack {
            Form {
                Section(L10n.tr("range.sheet.quick_precise", default: "实时短窗（精确到分钟）")) {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 68), spacing: 8)], spacing: 8) {
                        ForEach(quickWindows, id: \.label) { win in
                            Button {
                                let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
                                let startMs = nowMs - Int64(win.seconds * 1000)
                                Task {
                                    await store.applyPreciseTimeRange(startMs: startMs, endMs: nil)
                                    dismiss()
                                }
                            } label: {
                                Text(win.label)
                                    .font(.caption.weight(.medium))
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 7)
                                    .background(SemanticColors.surfaceSubtle, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.vertical, 4)
                }

                Picker(L10n.tr("range.sheet.mode", default: "选择方式"), selection: $mode) {
                    Text(L10n.tr("range.sheet.mode.date", default: "按日期")).tag(0)
                    Text(L10n.tr("range.sheet.mode.precise", default: "精确时刻")).tag(1)
                }
                .pickerStyle(.segmented)

                if mode == 0 {
                    Section(L10n.tr("range.sheet.date_bounds", default: "日期范围")) {
                        DatePicker(
                            L10n.tr("range.sheet.start", default: "开始日期"),
                            selection: $startDate,
                            displayedComponents: .date
                        )
                        DatePicker(
                            L10n.tr("range.sheet.end", default: "结束日期"),
                            selection: $endDate,
                            displayedComponents: .date
                        )
                    }
                } else {
                    Section(L10n.tr("range.sheet.precise_bounds", default: "精确起止时刻")) {
                        DatePicker(
                            L10n.tr("range.sheet.start_time", default: "开始时刻"),
                            selection: $preciseStart,
                            displayedComponents: [.date, .hourAndMinute]
                        )
                        Toggle(L10n.tr("range.sheet.follow_now", default: "结束时刻锚定现在"), isOn: $followNow)
                        if !followNow {
                            DatePicker(
                                L10n.tr("range.sheet.end_time", default: "结束时刻"),
                                selection: $preciseEnd,
                                displayedComponents: [.date, .hourAndMinute]
                            )
                        }
                    }
                }
            }
            .navigationTitle(L10n.tr("range.sheet.title", default: "自定义时间范围"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.tr("pairing.alert.cancel", default: "取消")) {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.tr("range.sheet.apply", default: "应用")) {
                        Task {
                            if mode == 0 {
                                await store.applyCustomDateRange(start: startDate, end: endDate)
                            } else {
                                let startMs = Int64(preciseStart.timeIntervalSince1970 * 1000)
                                let endMs: Int64? = followNow ? nil : Int64(preciseEnd.timeIntervalSince1970 * 1000)
                                await store.applyPreciseTimeRange(startMs: startMs, endMs: endMs)
                            }
                            dismiss()
                        }
                    }
                }
            }
        }
    }
}
