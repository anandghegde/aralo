import AraloBridge
import Foundation

extension AraloService {
    /// The embedding model search by meaning runs on (PRD A4, ADR-0016), which
    /// `make app` puts inside the app. Nil in a build made without it, and in
    /// tests: search then goes by words alone.
    static var bundledModel: URL? {
        Bundle.main.url(forResource: "potion-base-8M", withExtension: nil)
    }

    /// Hands the core the model the app ships with. The core loads it and
    /// embeds the library on its indexer thread, so nothing here waits, and
    /// nothing of the library leaves the Mac.
    func useBundledModel(_ core: Core) {
        guard let model = Self.bundledModel else { return }
        core.useModel(folder: model.path)
    }
}
