import AraloBridge
import Foundation

extension Error {
    /// What went wrong, in words to show. The bridge's own description is the
    /// Swift case spelled out, which is no use on screen; inside it is the
    /// sentence the core wrote.
    var reason: String {
        switch self {
        case let bridge as BridgeError:
            switch bridge {
            case .Library(let message), .CompatTable(let message), .Import(let message):
                return message
            }
        default:
            return localizedDescription
        }
    }
}
