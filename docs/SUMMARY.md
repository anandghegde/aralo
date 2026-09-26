# Summary

[Aralo](introduction.md)

# Using Aralo

- [Installing](installing.md)
- [The library format](format/README.md)
  - [The library](format/library.md)
  - [Snippets](format/snippet.md)
  - [Placeholders](format/placeholders.md)
  - [Matching](format/matching.md)
  - [The expansion plan](format/expansion-plan.md)
  - [Importing](format/import.md)
- [AI endpoints that pass](compatibility.md)

# Privacy and security

- [Data flow](data-flow.md)
- [Threat model](threat-model.md)
- [Reporting a vulnerability](security.md)

# Contributing

- [Contributing](contributing.md)
- [Architecture](architecture.md)
- [Writing a provider adapter](adapters.md)
- [Releasing](releasing.md)
- [Progress](progress.md)
- [Decision records](adr/README.md)
  - [0001 Rust core, Swift shell](adr/0001-rust-core-native-swift-shell.md)
  - [0002 UniFFI bridge](adr/0002-uniffi-bridge.md)
  - [0003 One process](adr/0003-single-process-architecture.md)
  - [0004 Threading model](adr/0004-threading-model.md)
  - [0005 Engine purity](adr/0005-engine-crate-purity.md)
  - [0006 Files as the source of truth](adr/0006-files-as-source-of-truth.md)
  - [0007 AI gateway and network guard](adr/0007-ai-gateway-and-network-guard.md)
  - [0008 Library choices](adr/0008-library-choices.md)
  - [0009 Apache-2.0](adr/0009-apache-2-licence.md)
  - [0010 Minimum macOS 14](adr/0010-minimum-macos-14.md)
  - [0011 Shell defaults](adr/0011-shell-defaults.md)
  - [0012 Injection and input defaults](adr/0012-injection-and-input-defaults.md)
  - [0013 Import and export](adr/0013-import-and-export.md)
  - [0014 Clock and locale](adr/0014-clock-and-locale.md)
  - [0015 The format grows for the importer](adr/0015-format-grows-for-the-importer.md)
  - [0016 Static embeddings](adr/0016-static-embeddings.md)
