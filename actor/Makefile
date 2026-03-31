.PHONY: build build-dev build-release cid show-cid serve publish release clean distclean

HOME_RS := $(shell find src -type f -name '*.rs')
DID_MA_RS := $(shell find ../../ma/src -type f -name '*.rs')
MA_CORE_RS := $(shell find ../core/src -type f -name '*.rs')
BUILD_INPUTS := Makefile Cargo.toml Cargo.lock ../../ma/Cargo.toml ../../ma/Cargo.lock ../core/Cargo.toml $(HOME_RS) $(DID_MA_RS) $(MA_CORE_RS)

PKG_STAMP := .build/pkg.stamp
PKG_DEV_STAMP := .build/pkg-dev.stamp
CID_FILE := .cid
IPNS_ALIAS := ma-actor
SOURCE_DATE_EPOCH ?= $(shell git log -1 --format=%ct 2>/dev/null || date +%s)


build: $(PKG_DEV_STAMP) cid

build-dev: $(PKG_DEV_STAMP)

build-release: $(PKG_STAMP)

$(PKG_STAMP): $(BUILD_INPUTS)
	mkdir -p .build
	SOURCE_DATE_EPOCH=$(SOURCE_DATE_EPOCH) \
	CARGO_INCREMENTAL=0 \
	RUSTFLAGS="--remap-path-prefix=$(CURDIR)=." \
	wasm-pack build --release --target web --out-dir www/pkg
	touch $(PKG_STAMP)

cid: $(PKG_DEV_STAMP)
	@set -e; \
	cid=$$(ipfs add -Qr www); \
	echo "$$cid" > $(CID_FILE); \
	echo "Wrote actor web CID to $(CID_FILE): $$cid"

show-cid:
	@if [ -f $(CID_FILE) ]; then \
		cat $(CID_FILE); \
	else \
		echo "$(CID_FILE) not found. Run: make build"; \
		exit 1; \
	fi

$(PKG_DEV_STAMP): $(BUILD_INPUTS)
	mkdir -p .build
	wasm-pack build --dev --target web --out-dir www/pkg
	touch $(PKG_DEV_STAMP)

serve: build
	cd www && python3 -m http.server 8081

release:
	$(MAKE) --no-print-directory -B build-release
	@set -e; \
	alias_name="$(IPNS_ALIAS)"; \
	key_id=$$(ipfs key list -l | awk '$$2=="'"$$alias_name"'" {print $$1; exit}'); \
	if [ -z "$$key_id" ]; then \
		echo "IPNS key '$$alias_name' not found; creating it..."; \
		ipfs key gen --type=ed25519 "$$alias_name" >/dev/null; \
		key_id=$$(ipfs key list -l | awk '$$2=="'"$$alias_name"'" {print $$1; exit}'); \
	fi; \
	if [ -z "$$key_id" ]; then \
		echo "Could not resolve key id for '$$alias_name'."; \
		exit 1; \
	fi; \
	cid=$$(ipfs add -Qr www); \
	ipfs name publish --key="$$alias_name" "/ipfs/$$cid" >/dev/null; \
	echo "Published ma-actor webapp to IPNS (release build):"; \
	echo "  Key alias: $$alias_name"; \
	echo "  IPNS: /ipns/$$key_id"; \
	echo "  CID: /ipfs/$$cid"; \
	echo "  Published gateway URL: http://127.0.0.1:8080/ipns/$$key_id/"; \
	echo "  Runtime URL (required for Kubo API features): http://127.0.0.1:8081"

publish:
	$(MAKE) --no-print-directory -B build-dev
	@set -e; \
	alias_name="$(IPNS_ALIAS)"; \
	key_id=$$(ipfs key list -l | awk '$$2=="'"$$alias_name"'" {print $$1; exit}'); \
	if [ -z "$$key_id" ]; then \
		echo "IPNS key '$$alias_name' not found; creating it..."; \
		ipfs key gen --type=ed25519 "$$alias_name" >/dev/null; \
		key_id=$$(ipfs key list -l | awk '$$2=="'"$$alias_name"'" {print $$1; exit}'); \
	fi; \
	if [ -z "$$key_id" ]; then \
		echo "Could not resolve key id for '$$alias_name'."; \
		exit 1; \
	fi; \
	cid=$$(ipfs add -Qr www); \
	ipfs name publish --key="$$alias_name" "/ipfs/$$cid" >/dev/null; \
	echo "Published ma-actor webapp to IPNS (dev build, non-release):"; \
	echo "  Key alias: $$alias_name"; \
	echo "  IPNS: /ipns/$$key_id"; \
	echo "  CID: /ipfs/$$cid"; \
	echo "  Published gateway URL: http://127.0.0.1:8080/ipns/$$key_id/"; \
	echo "  Runtime URL (required for Kubo API features): http://127.0.0.1:8081"

clean:
	rm -rf www/pkg
	rm -rf pkg
	rm -rf .build

distclean: clean
	cargo clean
	rm -f $(CID_FILE)
