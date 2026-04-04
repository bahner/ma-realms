.PHONY: help core-build actor-build world-build actor-cid run-world dev check clean distclean

WORLD_SLUG ?= ma
WORLD_LISTEN ?=
WORLD_KUBO_URL ?=

ifeq ($(origin MA_REALMS_VERSION), undefined)
MA_REALMS_VERSION := dev-$(shell date +%s)
endif

ifeq ($(origin MA_WORLD_VERSION), undefined)
MA_WORLD_VERSION := $(MA_REALMS_VERSION)
endif

ifeq ($(origin MA_ACTOR_VERSION), undefined)
MA_ACTOR_VERSION := $(MA_REALMS_VERSION)
endif

help:
	@echo "ma-realms targets:"
	@echo "  make core-build                              Build ma-core"
	@echo "  make actor-build                             Build ma-actor web bundle and write actor/.cid"
	@echo "  make world-build                             Build ma-world"
	@echo "  make actor-cid                               Print actor/.cid"
	@echo "  make run-world WORLD_SLUG=<slug> [WORLD_LISTEN=ip:port] [WORLD_KUBO_URL=url]"
	@echo "  make dev                                     Alias for run-world"
	@echo "  make check                                   cargo check workspace"
	@echo "  make clean                                   Clean sub-crate build artifacts"
	@echo "  make distclean                               Deep clean across sub-crates"

core-build:
	$(MAKE) -C core build

actor-build:
	$(MAKE) -C actor build MA_ACTOR_VERSION="$(MA_ACTOR_VERSION)"

world-build:
	$(MAKE) -C world build MA_WORLD_VERSION="$(MA_WORLD_VERSION)"

actor-cid:
	$(MAKE) -C actor show-cid

run-world: core-build actor-build world-build
	@set -e; \
	cid=$$(cat actor/.cid); \
	args="run --world-slug $(WORLD_SLUG) --cid $$cid"; \
	if [ -n "$(WORLD_LISTEN)" ]; then \
		args="$$args --listen $(WORLD_LISTEN)"; \
	fi; \
	if [ -n "$(WORLD_KUBO_URL)" ]; then \
		args="$$args --kubo-url $(WORLD_KUBO_URL)"; \
	fi; \
	echo "Starting ma-world with actor CID=$$cid"; \
	echo "MA_REALMS_VERSION=$(MA_REALMS_VERSION)"; \
	echo "MA_WORLD_VERSION=$(MA_WORLD_VERSION)"; \
	echo "MA_ACTOR_VERSION=$(MA_ACTOR_VERSION)"; \
	echo "Command: cargo run --manifest-path world/Cargo.toml -- $$args"; \
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" cargo run --manifest-path world/Cargo.toml -- $$args

dev: run-world

check:
	cargo check -q

clean:
	$(MAKE) -C core clean
	$(MAKE) -C actor clean
	$(MAKE) -C world clean

distclean:
	$(MAKE) -C core distclean
	$(MAKE) -C actor distclean
	$(MAKE) -C world distclean
