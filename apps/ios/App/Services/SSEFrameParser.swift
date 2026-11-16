import Foundation

public struct SSEEventFrame: Equatable, Sendable {
    public let event: String
    public let data: String
    public let id: String?

    public init(event: String, data: String, id: String? = nil) {
        self.event = event
        self.data = data
        self.id = id
    }
}

/// Incremental WHATWG Server-Sent Events parser safe against partial UTF-8 chunks and empty heartbeat frames.
public struct SSEFrameParser: Sendable {
    private var lineBuffer: [UInt8] = []
    private var previousWasCarriageReturn = false
    private var currentEvent = "message"
    private var currentDataLines: [String] = []
    private var currentId: String?

    public init() {}

    public mutating func feed(data: Data) -> [SSEEventFrame] {
        var emitted: [SSEEventFrame] = []
        for byte in data {
            if let frame = feedByte(byte) {
                emitted.append(frame)
            }
        }
        return emitted
    }

    public mutating func feedByte(_ byte: UInt8) -> SSEEventFrame? {
        if byte == 0x0A { // \n
            if previousWasCarriageReturn {
                previousWasCarriageReturn = false
                return nil
            }
            return completeLine()
        }
        previousWasCarriageReturn = false
        if byte == 0x0D { // \r
            previousWasCarriageReturn = true
            return completeLine()
        }
        lineBuffer.append(byte)
        return nil
    }

    public mutating func finish() -> SSEEventFrame? {
        if !lineBuffer.isEmpty {
            if let frame = completeLine() {
                return frame
            }
        }
        return flushEvent()
    }

    private mutating func completeLine() -> SSEEventFrame? {
        let lineData = Data(lineBuffer)
        lineBuffer.removeAll(keepingCapacity: true)
        guard let line = String(data: lineData, encoding: .utf8) else {
            return nil
        }
        if line.isEmpty {
            return flushEvent()
        }
        if line.hasPrefix(":") {
            // Heartbeat/comment frame resets keepalive without dispatching an event
            return nil
        }

        let field: String
        let value: String
        if let colonIdx = line.firstIndex(of: ":") {
            field = String(line[..<colonIdx])
            let afterColon = line.index(after: colonIdx)
            if afterColon < line.endIndex, line[afterColon] == " " {
                value = String(line[line.index(after: afterColon)...])
            } else {
                value = String(line[afterColon...])
            }
        } else {
            field = line
            value = ""
        }

        switch field {
        case "event":
            currentEvent = value.isEmpty ? "message" : value
        case "data":
            currentDataLines.append(value)
        case "id":
            if !value.contains("\0") {
                currentId = value
            }
        default:
            break
        }
        return nil
    }

    private mutating func flushEvent() -> SSEEventFrame? {
        guard !currentDataLines.isEmpty else {
            currentEvent = "message"
            return nil
        }
        let payload = currentDataLines.joined(separator: "\n")
        let frame = SSEEventFrame(event: currentEvent, data: payload, id: currentId)
        currentEvent = "message"
        currentDataLines.removeAll(keepingCapacity: true)
        return frame
    }
}
