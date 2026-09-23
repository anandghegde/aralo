import Foundation

/// One key press with its modifiers, written "cmd+a", "ctrl+u" or "tab".
public struct KeyChord: Equatable, Sendable, CustomStringConvertible {
    public enum Key: Equatable, Sendable {
        case character(Character)
        case space
        case tab
        case `return`
        case delete
        case escape
    }

    public enum Modifier: String, CaseIterable, Sendable {
        case cmd
        case ctrl
        case opt
        case shift
    }

    public var key: Key
    public var modifiers: Set<Modifier>

    public init(_ key: Key, _ modifiers: Set<Modifier> = []) {
        self.key = key
        self.modifiers = modifiers
    }

    public static let selectAll = KeyChord(.character("a"), [.cmd])
    public static let copy = KeyChord(.character("c"), [.cmd])
    public static let undo = KeyChord(.character("z"), [.cmd])
    public static let killLine = KeyChord(.character("u"), [.ctrl])
    public static let delete = KeyChord(.delete)
    public static let `return` = KeyChord(.return)

    /// Nil for anything that is not modifiers, then one key.
    public init?(_ text: String) {
        var parts = text.lowercased().split(separator: "+", omittingEmptySubsequences: false).map(String.init)
        guard let last = parts.popLast(), let key = Self.key(named: last) else { return nil }
        var modifiers: Set<Modifier> = []
        for part in parts {
            guard let modifier = Modifier(rawValue: part), modifiers.insert(modifier).inserted else { return nil }
        }
        self.init(key, modifiers)
    }

    private static func key(named name: String) -> Key? {
        switch name {
        case "space": .space
        case "tab": .tab
        case "return", "enter": .return
        case "delete", "backspace": .delete
        case "escape", "esc": .escape
        default: name.count == 1 ? name.first.map(Key.character) : nil
        }
    }

    public var description: String {
        let name = switch key {
        case .character(let character): String(character)
        case .space: "space"
        case .tab: "tab"
        case .return: "return"
        case .delete: "delete"
        case .escape: "escape"
        }
        let held = Modifier.allCases.filter(modifiers.contains).map(\.rawValue)
        return (held + [name]).joined(separator: "+")
    }
}
