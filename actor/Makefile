.PHONY: build build-dev build-release cid show-cid serve publish release test-web clean distclean

HOME_RS := $(shell find src -type f -name '*.rs')
DID_MA_RS := $(shell find ../../ma/src -type f -name '*.rs')
MA_CORE_RS := $(shell find ../core/src -type f -name '*.rs')
BUILD_INPUTS := Makefile Cargo.toml Cargo.lock ../../ma/Cargo.toml ../../ma/Cargo.lock ../core/Cargo.toml $(HOME_RS) $(DID_MA_RS) $(MA_CORE_RS)
WEB_JS_SOURCES := $(shell find www -type f \( -name '*.js' -o -name '*.mjs' \) 2>/dev/null)
TEST_WEB_INPUTS := $(BUILD_INPUTS) $(WEB_JS_SOURCES)

CID_FILE := .cid
IPNS_ALIAS := ma-actor
SOURCE_DATE_EPOCH ?= $(shell git log -1 --format=%ct 2>/dev/null || date +%s)
MA_ACTOR_VERSION ?=
MA_ACTOR_VERSION_EFFECTIVE := $(strip $(MA_ACTOR_VERSION))
ifeq ($(MA_ACTOR_VERSION_EFFECTIVE),)
MA_ACTOR_VERSION_EFFECTIVE := $(shell printf 'dev-%s' "$$(date +%s)")
endif
STAMP_VERSION := $(shell printf '%s' "$(MA_ACTOR_VERSION_EFFECTIVE)" | tr -c 'A-Za-z0-9._-' '_')
PKG_STAMP := .build/pkg.$(STAMP_VERSION).stamp
PKG_DEV_STAMP := .build/pkg-dev.$(STAMP_VERSION).stamp
TEST_WEB_STAMP := .build/test-web.stamp


build: $(PKG_DEV_STAMP) cid

build-dev: $(PKG_DEV_STAMP)

build-release: $(PKG_STAMP)

$(PKG_STAMP): $(BUILD_INPUTS) $(TEST_WEB_STAMP)
	mkdir -p .build
	SOURCE_DATE_EPOCH=$(SOURCE_DATE_EPOCH) \
	MA_ACTOR_VERSION=$(MA_ACTOR_VERSION_EFFECTIVE) \
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

$(PKG_DEV_STAMP): $(BUILD_INPUTS) $(TEST_WEB_STAMP)
	mkdir -p .build
	MA_ACTOR_VERSION=$(MA_ACTOR_VERSION_EFFECTIVE) wasm-pack build --dev --target web --out-dir www/pkg
	touch $(PKG_DEV_STAMP)

$(TEST_WEB_STAMP): $(TEST_WEB_INPUTS)
	@set -e; \
	mkdir -p .build; \
	tests=$$(find www -type f \( -name '*.test.mjs' -o -name '*.test.js' -o -name '*.spec.mjs' -o -name '*.spec.js' \)); \
	if [ -z "$$tests" ]; then \
		echo "No web tests found under www/."; \
		touch $(TEST_WEB_STAMP); \
		exit 0; \
	fi; \
	if command -v node >/dev/null 2>&1; then \
		node --experimental-default-type=module --test $$tests; \
	elif command -v nodejs >/dev/null 2>&1; then \
		nodejs --experimental-default-type=module --test $$tests; \
	else \
		echo "Node.js runtime not found (need 'node' or 'nodejs') to run web regression tests."; \
		exit 1; \
	fi; \
	touch $(TEST_WEB_STAMP)

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
	echo "  Runtime URL (required for IPFS API features): http://127.0.0.1:8081"

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
	echo "  Runtime URL (required for IPFS API features): http://127.0.0.1:8081"

test-web:
	@$(MAKE) --no-print-directory $(TEST_WEB_STAMP)

clean:
	rm -rf www/pkg
	rm -rf pkg
	rm -rf .build

distclean: clean
	cargo clean
	rm -f $(CID_FILE)
