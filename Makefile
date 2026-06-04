.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	install -m 755 target/release/clear-typeface-interface ~/.local/bin/clear-typeface-interface

run:
	cargo run

clean:
	cargo clean
