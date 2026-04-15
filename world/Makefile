.PHONY: build clean distclean run serve lang_cid help

BINARY := target/debug/ma-world
RELEASE_BINARY := target/release/ma-world
STAMP := target/.build_stamp
MA_WORLD_VERSION ?=
RELEASE_OPTS ?= --release --target x86_64-unknown-linux-musl

help:
	@echo "Available targets:"
	@echo "  make build        - Build the world server (debug)"
	@echo "  make release      - Build optimized release binary"
	@echo "  make run          - Build and run the world server"
	@echo "  make .lang_cid    - Rebuild world default language CID file from lang/*.ftl"
	@echo "  make lang_cid     - Publish lang/*.ftl and generate DAG-CBOR language map CID"
	@echo "  make clean        - Remove build artifacts"
	@echo "  make distclean    - Clean + cargo clean"

build: $(STAMP)

$(STAMP): Cargo.toml src/*.rs ../../did/src/*.rs ../core/src/*.rs
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" cargo build $(BUILD_OPTS)
	@mkdir -p $(dir $(STAMP))
	@touch $(STAMP)

release: Cargo.toml src/*.rs ../../did/src/*.rs ../core/src/*.rs
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" cargo build $(RELEASE_OPTS)
	@echo "🏗️  Release binary at $(RELEASE_BINARY)"

run: build
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" ./$(BINARY)

.lang_cid: lang/*.ftl scripts/publish-lang-map.sh
	./scripts/publish-lang-map.sh

lang_cid:
	$(MAKE) .lang_cid

clean:
	rm -f $(STAMP)
	rm -rf target/debug/deps/libma_world* target/debug/ma-world

distclean:
	cargo clean

.PHONY: build clean distclean run help
