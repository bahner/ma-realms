.PHONY: all clean distclean

all:
	$(MAKE) -C core build
	$(MAKE) -C actor build
	$(MAKE) -C agent build
	$(MAKE) -C world build

clean:
	$(MAKE) -C core clean
	$(MAKE) -C actor clean
	$(MAKE) -C agent clean
	$(MAKE) -C world clean

distclean:
	$(MAKE) -C core distclean
	$(MAKE) -C actor distclean
	$(MAKE) -C agent distclean
	$(MAKE) -C world distclean

# These are developer targets for me.
# Add your own hosts in .ssh/config if you want
push: push-actor push-world

push-actor:
	$(MAKE) -C actor build-release
	scp -r actor/www ma-actor:/home/ma/

push-world:
	$(MAKE) -C world release
	scp target/x86_64-unknown-linux-musl/release/ma-world ma-world:bin/

.PHONY: all clean distclean push*
