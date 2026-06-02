.PHONY: build test deploy

build:
	cargo build --release

test:
	cargo test

deploy: build
	cp target/release/mx /tmp/mx-deploy-staging
	sudo /home/gabaum10/.soren/bin/deploy-mx
