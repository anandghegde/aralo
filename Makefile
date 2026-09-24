# Aralo's build entry points. `make help` lists them.
#
# The Rust workspace needs nothing from here: `cargo test --workspace` works on
# macOS, Linux and Windows. These targets exist for the Mac app, which needs
# the Rust core built as an XCFramework before Swift can see it.

MACOS     := apps/macos
KIT       := $(MACOS)/Packages/AraloKit
HARNESS   := $(MACOS)/Packages/AraloHarness
GENERATED := $(MACOS)/Generated/AraloBridge
PROJECT   := $(MACOS)/Aralo.xcodeproj
DERIVED   := target/xcode
APP       := $(DERIVED)/Build/Products/Debug/Aralo.app
# "-" signs ad hoc, and macOS then forgets the app's permission grants at every
# rebuild. Name a certificate from your keychain to keep them:
# `make app SIGN_IDENTITY="My Self-Signed Cert"`.
SIGN_IDENTITY ?= -

.PHONY: help bootstrap xcframework xcframework-debug project model app run test test-swift lint lint-swift check \
	matrix latency clean

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-18s %s\n", $$1, $$2}'

bootstrap: xcframework project ## Everything the Mac app needs: XCFramework, Swift bindings, Xcode project

xcframework: ## Build the Rust core as an XCFramework, with its Swift bindings (release)
	scripts/build-xcframework.sh

xcframework-debug: ## The same, unoptimised: much faster to build
	scripts/build-xcframework.sh --debug

project: ## Generate apps/macos/Aralo.xcodeproj from project.yml
	cd $(MACOS) && xcodegen generate --quiet

model: ## Fetch the embedding model search by meaning ships with, into models/
	scripts/fetch-model.sh

# The model comes before the project: xcodegen adds it only if it is there.
app: $(GENERATED) model project ## Build Aralo.app (Debug, signed ad hoc), with the model inside it
	xcodebuild -project $(PROJECT) -scheme Aralo -configuration Debug \
		-derivedDataPath $(DERIVED) -quiet CODE_SIGN_IDENTITY="$(SIGN_IDENTITY)" build
	@echo "Built $(APP)"

run: app ## Build and launch the app
	open $(APP)

test: ## Rust tests, all crates
	cargo test --workspace

test-swift: $(GENERATED) ## Swift tests, which call the real Rust core
	swift test --package-path $(KIT)
	swift test --package-path $(HARNESS)

lint: ## rustfmt and clippy, as CI runs them
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

lint-swift: ## SwiftLint, strict
	cd $(MACOS) && swiftlint --strict --quiet

check: lint test ## What CI's Rust jobs run, plus the dependency rules
	scripts/check-deps.sh

# The two targets below take over the keyboard of the Mac they run on: they
# launch apps and type into them with real key events. Run them at a Mac set
# aside for it, with Accessibility granted to the terminal and to Aralo.app.
# ARGS passes options through: `make matrix ARGS="--only com.apple.TextEdit"`.
matrix: app ## Injection matrix: type every case into every app of the table (takes over the keyboard)
	swift run --package-path $(HARNESS) aralo-harness matrix --aralo $(APP) $(ARGS)

latency: app ## Typed-to-inserted latency in TextEdit, p50/p95/p99 (takes over the keyboard)
	swift run --package-path $(HARNESS) aralo-harness latency --aralo $(APP) $(ARGS)

clean: ## Remove build output, including the generated bridge and project
	cargo clean
	rm -rf $(MACOS)/Generated $(PROJECT) $(KIT)/.build $(HARNESS)/.build

# The bridge is generated; build it on demand for targets that need it.
$(GENERATED):
	scripts/build-xcframework.sh
