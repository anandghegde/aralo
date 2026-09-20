import AraloBridge
import Foundation

public enum CaseStatus: String, Codable, Sendable {
    case pass
    /// Passed, but not at the first attempt.
    case flaky
    case fail
    /// Waits on a later milestone.
    case pending
    /// Another app took the keyboard, so no more keys were sent.
    case focusLost = "focus-lost"
}

public enum ReadBack: String, Codable, Sendable {
    case accessibility
    case copy
}

/// One case in one app. It holds verdicts and numbers, and never the text that
/// was read back: a terminal's scroll-back is not the harness's to keep.
public struct CaseResult: Codable, Equatable, Sendable {
    public var matrixCase: MatrixCase
    public var status: CaseStatus
    public var attempts: Int
    public var method: String?
    public var readBack: ReadBack?
    /// Delimiter posted to text seen, at the first attempt only.
    public var latencyMs: Double?
    public var reason: String?

    enum CodingKeys: String, CodingKey {
        case matrixCase = "case"
        case status, attempts, method
        case readBack = "read_back"
        case latencyMs = "latency_ms"
        case reason
    }

    public init(
        _ matrixCase: MatrixCase, _ status: CaseStatus, attempts: Int = 0, method: String? = nil,
        readBack: ReadBack? = nil, latencyMs: Double? = nil, reason: String? = nil
    ) {
        self.matrixCase = matrixCase
        self.status = status
        self.attempts = attempts
        self.method = method
        self.readBack = readBack
        self.latencyMs = latencyMs
        self.reason = reason
    }

    public var passed: Bool { status == .pass || status == .flaky }
    public var counts: Bool { status != .pending }
}

public struct AppResult: Codable, Equatable, Sendable {
    public var name: String
    public var bundleID: String
    /// Nil when the cases ran; otherwise why they did not.
    public var skipped: String?
    public var cases: [CaseResult]

    enum CodingKeys: String, CodingKey {
        case name
        case bundleID = "bundle_id"
        case skipped, cases
    }

    public init(name: String, bundleID: String, skipped: String? = nil, cases: [CaseResult] = []) {
        self.name = name
        self.bundleID = bundleID
        self.skipped = skipped
        self.cases = cases
    }

    public var tested: Bool { skipped == nil }
    public var passed: Bool { tested && cases.filter(\.counts).allSatisfy(\.passed) && cases.contains(where: \.counts) }
}
