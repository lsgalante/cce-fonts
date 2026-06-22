.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	@if [ -f ../target/release/cce-fonts ]; then \
		install -m 755 ../target/release/cce-fonts ~/.local/bin/cce-fonts; \
	elif [ -f target/release/cce-fonts ]; then \
		install -m 755 target/release/cce-fonts ~/.local/bin/cce-fonts; \
	else \
		echo "Error: cce-fonts binary not found"; exit 1; \
	fi

run:
	cargo run

clean:
	cargo clean
