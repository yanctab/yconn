# Makefile

.PHONY: build install setup lint fmt fmt-check test coverage clean release package docs publish help

BINARY := $(shell grep '^name' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*= "//' | sed 's/"//')
TARGET := x86_64-unknown-linux-musl
PREFIX ?= /usr/local

## help - show available targets
help:
	@grep -E '^## [a-zA-Z_-]+ - ' Makefile | awk 'BEGIN {FS=" - "} {printf "  %-15s %s\n", substr($$1, 4), $$2}'

## setup - install Rust toolchain components, cargo-llvm-cov, and system deps (distro auto-detected)
setup:
	./scripts/install-deps.sh
	rustup component add rustfmt clippy llvm-tools-preview
	rustup target add x86_64-unknown-linux-musl
	cargo install cargo-llvm-cov --locked

## build - compile a static musl release binary
build:
	cargo build --release --target $(TARGET)

## install - build a distro-native package and install it (Debian/Ubuntu or Arch)
install: package
	@if [ ! -f /etc/os-release ]; then \
		echo "error: cannot detect distribution — /etc/os-release not found."; \
		echo ""; \
		echo "You can build packages without installing by running:"; \
		echo "  make package"; \
		echo ""; \
		echo "Then install manually:"; \
		echo "  Debian/Ubuntu: sudo apt-get install ./dist/$(BINARY)_$(VERSION)_amd64.deb"; \
		echo "  Arch Linux:    sudo pacman -U dist/$(BINARY)-$(VERSION)-1-x86_64.pkg.tar.zst"; \
		exit 1; \
	fi; \
	. /etc/os-release; \
	ID="$${ID:-}"; \
	ID_LIKE="$${ID_LIKE:-}"; \
	is_debian_like() { \
		case "$$ID" in debian|ubuntu) return 0;; esac; \
		case "$$ID_LIKE" in *debian*|*ubuntu*) return 0;; esac; \
		return 1; \
	}; \
	is_arch_like() { \
		case "$$ID" in arch) return 0;; esac; \
		case "$$ID_LIKE" in *arch*) return 0;; esac; \
		return 1; \
	}; \
	if is_debian_like; then \
		echo "Detected Debian/Ubuntu-based distribution: $$ID"; \
		sudo apt-get install -y ./dist/$(BINARY)_$(VERSION)_amd64.deb; \
	elif is_arch_like; then \
		echo "Detected Arch Linux-based distribution: $$ID"; \
		sudo pacman -U --noconfirm dist/$(BINARY)-$(VERSION)-1-x86_64.pkg.tar.zst; \
	else \
		echo "error: unsupported distribution: $$ID"; \
		echo ""; \
		echo "You can build packages without installing by running:"; \
		echo "  make package"; \
		echo ""; \
		echo "Then install manually:"; \
		echo "  Debian/Ubuntu: sudo apt-get install ./dist/$(BINARY)_$(VERSION)_amd64.deb"; \
		echo "  Arch Linux:    sudo pacman -U dist/$(BINARY)-$(VERSION)-1-x86_64.pkg.tar.zst"; \
		exit 1; \
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

## test - run the test suite and print coverage summary
test:
	cargo test
	$(MAKE) coverage

## coverage - print code coverage summary (requires cargo-llvm-cov)
coverage:
	@command -v cargo-llvm-cov >/dev/null 2>&1 || (echo "cargo-llvm-cov not found — run: cargo install cargo-llvm-cov --locked" && exit 1)
	cargo llvm-cov --summary-only

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
