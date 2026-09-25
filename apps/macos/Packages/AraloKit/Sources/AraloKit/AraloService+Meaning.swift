import AraloBridge
import Foundation

extension AraloService {
    /// The embedding model search by meaning runs on (PRD A4, ADR-0016), which
    /// `make app` puts inside the app. Nil in a build made without it, and in
    /// tests: search then goes by words alone.
    static var bundledModel: URL? {
        Bundle.main.url(forResource: "potion-base-8M", withExtension: nil)
    }

    /// Hands the core the model the app ships with, and the AI settings whose
    /// switch it follows (plan 4.10): while AI is off the model is never
    /// loaded, and switching AI on or off in the settings loads or drops it.
    /// The core does the work on its indexer thread, so nothing here waits,
    /// and nothing of the library leaves the Mac.
    ///
    /// This is why a copy of Aralo that ships a model reads profiles.toml at
    /// start: the switch is in it. No key is read until a model is asked
    /// something.
    func useBundledModel(_ core: Core) {
        guard let model = Self.bundledModel, let settings = try? aiSettings() else { return }
        core.useModel(folder: model.path, ai: settings)
    }
}
