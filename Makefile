# Aralo's build entry points. `make help` lists them.
#
# The Rust workspace needs nothing from here: `cargo test --workspace` works on
# macOS, Linux and Windows. These targets exist for the Mac app, which needs
# the Rust core built as an XCFramework before Swift can see it.

MACOS     := apps/macos
KIT       := $(MACOS)/Packages/AraloKit
GENERATED := $(MACOS)/Generated/AraloBridge
PROJECT   := $(MACOS)/Aralo.xcodeproj
DERIVED   := target/xcode
APP       := $(DERIVED)/Build/Products/Debug/Aralo.app

.PHONY: help bootstrap xcframework xcframework-debug project app run test test-swift lint lint-swift check clean

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-18s %s\n", $$1, $$2}'

bootstrap: xcframework project ## Everything the Mac app needs: XCFramework, Swift bindings, Xcode project

xcframework: ## Build the Rust core as an XCFramework, with its Swift bindings (release)
	scripts/build-xcframework.sh

xcframework-debug: ## The same, unoptimised: much faster to build
	scripts/build-xcframework.sh --debug

project: ## Generate apps/macos/Aralo.xcodeproj from project.yml
	cd $(MACOS) && xcodegen generate --quiet

app: $(GENERATED) project ## Build Aralo.app (Debug, signed ad hoc)
	xcodebuild -project $(PROJECT) -scheme Aralo -configuration Debug \
		-derivedDataPath $(DERIVED) -quiet build
	@echo "Built $(APP)"

run: app ## Build and launch the app
	open $(APP)

test: ## Rust tests, all crates
	cargo test --workspace

test-swift: $(GENERATED) ## Swift tests, which call the real Rust core
	swift test --package-path $(KIT)

lint: ## rustfmt and clippy, as CI runs them
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

lint-swift: ## SwiftLint, strict
	cd $(MACOS) && swiftlint --strict --quiet

check: lint test ## What CI's Rust jobs run, plus the dependency rules
	scripts/check-deps.sh

clean: ## Remove build output, including the generated bridge and project
	cargo clean
	rm -rf $(MACOS)/Generated $(PROJECT) $(KIT)/.build

# The bridge is generated; build it on demand for targets that need it.
$(GENERATED):
	scripts/build-xcframework.sh
