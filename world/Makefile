.PHONY: build clean distclean run serve help

BINARY := target/debug/ma-world
RELEASE_BINARY := target/release/ma-world
STAMP := target/.build_stamp
MA_WORLD_VERSION ?=

help:
	@echo "Available targets:"
	@echo "  make build        - Build the world server (debug)"
	@echo "  make release      - Build optimized release binary"
	@echo "  make run          - Build and run the world server"
	@echo "  make clean        - Remove build artifacts"
	@echo "  make distclean    - Clean + cargo clean"

build: $(STAMP)

$(STAMP): Cargo.toml src/*.rs ../../did/src/*.rs ../core/src/*.rs
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" cargo build
	@mkdir -p $(dir $(STAMP))
	@touch $(STAMP)

release: Cargo.toml src/*.rs ../../did/src/*.rs ../core/src/*.rs
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" cargo build --release
	@echo "🏗️  Release binary at $(RELEASE_BINARY)"

run: build
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" ./$(BINARY)

clean:
	rm -f $(STAMP)
	rm -rf target/debug/deps/libma_world* target/debug/ma-world

distclean:
	cargo clean

.PHONY: build clean distclean run help
