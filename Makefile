# Makefile

.PHONY: build lint fmt fmt-check test clean release package docs publish

BINARY := $(shell grep '^name' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
TARGET := x86_64-unknown-linux-musl

build:
	cargo build --release --target $(TARGET)

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint:
	$(MAKE) fmt-check
	cargo clippy -- -D warnings

test:
	cargo test

clean:
	cargo clean

release:
	git tag v$(VERSION)
	git push origin v$(VERSION)

package:
	$(MAKE) build
	$(MAKE) build-deb
	$(MAKE) build-aur

docs:
	pandoc docs/man/$(BINARY).1.md -s -t man -o docs/man/$(BINARY).1

build-deb:
	@scripts/build-deb.sh $(BINARY) $(VERSION)

build-aur:
	@scripts/build-aur.sh $(BINARY) $(VERSION)

publish: lint test
	cargo publish
