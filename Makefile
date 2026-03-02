# Makefile

.PHONY: build lint fmt fmt-check test clean release package docs publish help

BINARY := $(shell grep '^name' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
TARGET := x86_64-unknown-linux-musl

## help - show available targets
help:
	@grep -E '^## [a-zA-Z_-]+ - ' Makefile | awk 'BEGIN {FS=" - "} {printf "  %-15s %s\n", substr($$1, 4), $$2}'

## build - compile a static musl release binary
build:
	cargo build --release --target $(TARGET)

## fmt - auto-format code with cargo fmt
fmt:
	cargo fmt

## fmt-check - check code formatting without modifying files
fmt-check:
	cargo fmt --check

## lint - check formatting and run clippy
lint:
	$(MAKE) fmt-check
	cargo clippy -- -D warnings

## test - run the test suite
test:
	cargo test

## clean - remove build artifacts
clean:
	cargo clean

## release - tag the current version in Cargo.toml and push to trigger the release pipeline
release:
	git tag v$(VERSION)
	git push origin v$(VERSION)

## package - build .deb and Arch .pkg.tar.zst from the release binary
package:
	$(MAKE) build
	$(MAKE) build-deb
	$(MAKE) build-pkg

## docs - generate man page from markdown source
docs:
	pandoc docs/man/$(BINARY).1.md -s -t man -o docs/man/$(BINARY).1

build-deb:
	@scripts/build-deb.sh $(BINARY) $(VERSION)

build-pkg:
	@scripts/build-pkg.sh $(BINARY) $(VERSION)

## publish - publish the crate to crates.io
publish: lint test
	cargo publish
