.PHONY: help build write-version dev mcp clean distclean

MA_AGENT_VERSION ?= $(shell printf 'dev-%s' "$$(date +%s)")
MA_AGENT_LISTEN ?=
MA_AGENT_KUBO_KEY_ALIAS ?=
MA_AGENTD_URL ?= http://127.0.0.1:5003
AGENT_VERSION_FILE := .generated/agent-version.txt

help:
	@echo "Available targets:"
	@echo "  make build         - Build ma-agent"
	@echo "  make write-version - Write .generated/agent-version.txt"
	@echo "  make dev           - Run ma-agent in daemon mode"
	@echo "  make mcp           - Run ma-agent MCP bridge"
	@echo "  make clean         - Remove local generated files"
	@echo "  make distclean     - Clean + cargo clean"

build:
	cargo build --bin ma-agent

write-version:
	@mkdir -p $(dir $(AGENT_VERSION_FILE))
	@printf "%s\n" "$(MA_AGENT_VERSION)" > $(AGENT_VERSION_FILE)
	@echo "Wrote $(AGENT_VERSION_FILE): $(MA_AGENT_VERSION)"

dev: write-version
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
	echo "Command: cargo run --bin ma-agent -- $$args"; \
	cargo run --bin ma-agent -- $$args

mcp:
	@set -e; \
	echo "Starting ma-agent MCP server"; \
	echo "MA_AGENTD_URL=$(MA_AGENTD_URL)"; \
	echo "Command: cargo run --bin ma-agent -- --mcp --agentd-url $(MA_AGENTD_URL)"; \
	cargo run --bin ma-agent -- --mcp --agentd-url "$(MA_AGENTD_URL)"

clean:
	rm -rf .generated

distclean: clean
	cargo clean
