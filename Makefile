default:
	@echo "Possible make targets:"
	@echo ""
	@echo "- debug: Build binary in debug mode"
	@echo "- release: Build stripped release binary"
	@echo "- release-unstripped: Build release binary with debug symbols"
	@echo "- release-jessie: Build a stripped release binary inside a Debian 8 Docker container"
	@echo ""
	@echo "Usage: make <target>"

debug:
	cargo build

release-unstripped:
	cargo build --release

release: release-unstripped
	strip target/release/smartmail

release-jessie:
	docker run --rm -v $(shell pwd):/code -w /code rust:1.20-jessie \
		/bin/bash -c "apt-get update && apt-get install -y -q libsodium-dev && make release"

.PHONY: default debug release-unstripped release release-jessie
