include env

all: deb

deb: src/main.rs src/bridge.rs olddeb
	cargo deb --target aarch64-unknown-linux-gnu

olddeb:
	cargo deb --target aarch64-unknown-linux-gnu --variant=oldlib

clean:
	-rm -r target

.PHONY: all clean deb
