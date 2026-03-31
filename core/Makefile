.PHONY: help build check test clean distclean

STAMP := target/.build_stamp

help:
	@echo "Available targets:"
	@echo "  make build      - Build ma-core (debug)"
	@echo "  make check      - cargo check"
	@echo "  make test       - cargo test"
	@echo "  make clean      - Remove local stamp"
	@echo "  make distclean  - cargo clean"

build: $(STAMP)

$(STAMP): Cargo.toml src/*.rs
	cargo build
	@mkdir -p target
	@touch $(STAMP)

check:
	cargo check -q

test:
	cargo test -q

clean:
	rm -f $(STAMP)

distclean:
	cargo clean
