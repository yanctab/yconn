# Makefile

.PHONY: build install lint fmt fmt-check test clean release package docs publish help

BINARY := $(shell grep '^name' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
TARGET := x86_64-unknown-linux-musl
PREFIX ?= /usr/local

## help - show available targets
help:
	@grep -E '^## [a-zA-Z_-]+ - ' Makefile | awk 'BEGIN {FS=" - "} {printf "  %-15s %s\n", substr($$1, 4), $$2}'

## build - compile a static musl release binary
build:
	cargo build --release --target $(TARGET)

## install - install binary (and man page if built) to PREFIX=/usr/local — use sudo for system paths
install: build
	install -Dm755 target/$(TARGET)/release/$(BINARY) $(PREFIX)/bin/$(BINARY)
	@if [ -f docs/man/$(BINARY).1 ]; then \
		install -Dm644 docs/man/$(BINARY).1 $(PREFIX)/share/man/man1/$(BINARY).1; \
	fi

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

## release - bump minor version, commit, tag, and push to trigger the release pipeline
release:
	@git fetch --tags
	@LATEST=$$(git tag --sort=-v:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$$' | head -1); \
	if [ -z "$$LATEST" ]; then echo "error: no semver tag found"; exit 1; fi; \
	MAJOR=$$(echo "$$LATEST" | sed 's/^v//' | cut -d. -f1); \
	MINOR=$$(echo "$$LATEST" | sed 's/^v//' | cut -d. -f2); \
	NEW_MINOR=$$((MINOR + 1)); \
	NEW_VERSION="$$MAJOR.$$NEW_MINOR.0"; \
	echo "Bumping $$LATEST -> v$$NEW_VERSION"; \
	sed -i "s/^version = \".*\"/version = \"$$NEW_VERSION\"/" Cargo.toml; \
	cargo update -p yconn; \
	git add Cargo.toml Cargo.lock; \
	git commit -m "yconn v$$NEW_VERSION"; \
	git tag "v$$NEW_VERSION"; \
	git push origin HEAD "v$$NEW_VERSION"

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
