.PHONY: help build test lint fmt clean

UNAME_S := $(shell uname -s)
BUILD_MACOS := $(CURDIR)/scripts/build-macos.sh

help:
	@echo "Available targets:"
	@echo "  make build  - Build the project"
	@echo "  make test   - Run tests"
	@echo "  make lint   - Run clippy with warnings denied"
	@echo "  make fmt    - Run rustfmt"
	@echo "  make clean  - Remove build artifacts"

build:
ifeq ($(UNAME_S),Darwin)
	@$(BUILD_MACOS) build --manifest-path $(CURDIR)/Cargo.toml
else
	cargo build
endif

test:
ifeq ($(UNAME_S),Darwin)
	@$(BUILD_MACOS) test --manifest-path $(CURDIR)/Cargo.toml
else
	cargo test
endif

lint:
ifeq ($(UNAME_S),Darwin)
	@$(BUILD_MACOS) clippy --manifest-path $(CURDIR)/Cargo.toml --all-targets -- -D warnings
else
	cargo clippy --all-targets -- -D warnings
endif

fmt:
ifeq ($(UNAME_S),Darwin)
	@$(BUILD_MACOS) fmt --all
else
	cargo fmt --all
endif

clean:
	cargo clean
