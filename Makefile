.PHONY: help core-build actor-build world-build actor-cid write-agent-version write-actor-version run-world dev dev-agent dev-agent-mcp check clean distclean

WORLD_SLUG ?= ma
WORLD_LISTEN ?=
WORLD_KUBO_URL ?=
MA_AGENT_LISTEN ?=
MA_AGENT_KUBO_KEY_ALIAS ?=
MA_AGENTD_URL ?= http://127.0.0.1:5003

MA_ACTOR_VERSION_ORIGIN := $(origin MA_ACTOR_VERSION)

AGENT_VERSION_FILE := agent/.generated/agent-version.txt
ACTOR_VERSION_FILE := actor/www/pkg/build-version.js
ACTOR_VERSION_JSON_FILE := actor/www/pkg/build-version.json

ifeq ($(origin MA_REALMS_VERSION), undefined)
MA_REALMS_VERSION := dev-$(shell date +%s)
endif

ifeq ($(origin MA_AGENT_VERSION), undefined)
MA_AGENT_VERSION := dev-$(shell date +%s)
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
	@echo "  make write-agent-version                     Write agent/.generated/agent-version.txt"
	@echo "  make write-actor-version                     Write actor/www/pkg/build-version.js when MA_ACTOR_VERSION is set"
	@echo "  make dev                                     Alias for run-world"
	@echo "  make dev-agent [MA_AGENT_LISTEN=ip:port] [MA_AGENT_KUBO_KEY_ALIAS=alias]"
	@echo "  make dev-agent-mcp [MA_AGENTD_URL=url]      Start MCP bridge for ma-agentd"
	@echo "  make check                                   cargo check workspace"
	@echo "  make clean                                   Clean sub-crate build artifacts"
	@echo "  make distclean                               Deep clean across sub-crates"

core-build:
	$(MAKE) -C core build

actor-build:
	$(MAKE) -C actor build MA_ACTOR_VERSION="$(MA_ACTOR_VERSION)"
	$(MAKE) --no-print-directory write-actor-version

world-build:
	$(MAKE) -C world build MA_WORLD_VERSION="$(MA_WORLD_VERSION)"

actor-cid:
	$(MAKE) -C actor show-cid

write-agent-version:
	@mkdir -p $(dir $(AGENT_VERSION_FILE))
	@printf "%s\n" "$(MA_AGENT_VERSION)" > $(AGENT_VERSION_FILE)
	@echo "Wrote $(AGENT_VERSION_FILE): $(MA_AGENT_VERSION)"

write-actor-version:
ifeq ($(MA_ACTOR_VERSION_ORIGIN), undefined)
	@echo "MA_ACTOR_VERSION is not set; skipping $(ACTOR_VERSION_FILE)"
else
	@mkdir -p $(dir $(ACTOR_VERSION_FILE))
	@printf "globalThis.MA_ACTOR_VERSION = '%s';\n" "$(MA_ACTOR_VERSION)" > $(ACTOR_VERSION_FILE)
	@printf '{\n  "ma_actor_version": "%s"\n}\n' "$(MA_ACTOR_VERSION)" > $(ACTOR_VERSION_JSON_FILE)
	@echo "Wrote $(ACTOR_VERSION_FILE): $(MA_ACTOR_VERSION)"
endif

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
	if [ -n "$$RUST_LOG" ]; then echo "RUST_LOG=$$RUST_LOG"; else echo "RUST_LOG=(unset; controlled by --log-level/MA_LOG_LEVEL)"; fi; \
	echo "Command: cargo run --manifest-path world/Cargo.toml -- $$args"; \
	MA_WORLD_VERSION="$(MA_WORLD_VERSION)" cargo run --manifest-path world/Cargo.toml -- $$args

dev: run-world

dev-agent: write-agent-version
	@set -e; \
	args="--daemon"; \
	if [ -n "$(MA_AGENT_LISTEN)" ]; then \
		args="$$args --listen $(MA_AGENT_LISTEN)"; \
	fi; \
	if [ -n "$(MA_AGENT_KUBO_KEY_ALIAS)" ]; then \
		args="$$args --kubo-key-alias $(MA_AGENT_KUBO_KEY_ALIAS)"; \
	fi; \
	echo "Starting ma-agentd"; \
	echo "MA_AGENT_VERSION=$(MA_AGENT_VERSION)"; \
	echo "Command: cargo run --manifest-path agent/Cargo.toml --bin ma-agent -- $$args"; \
	cargo run --manifest-path agent/Cargo.toml --bin ma-agent -- $$args

dev-agent-mcp:
	@set -e; \
	echo "Starting ma-agent MCP server"; \
	echo "MA_AGENTD_URL=$(MA_AGENTD_URL)"; \
	echo "Command: cargo run --manifest-path agent/Cargo.toml --bin ma-agent -- --mcp --agentd-url $(MA_AGENTD_URL)"; \
	cargo run --manifest-path agent/Cargo.toml --bin ma-agent -- --mcp --agentd-url "$(MA_AGENTD_URL)"

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
