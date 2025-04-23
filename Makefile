all: fetch-deps install test

fetch-deps:
	git submodule update --init --recursive

install:
	cd pear_frontend && cargo install --locked --path . 

test-pear:
	cd tests && cargo clean && cargo pear

test-scrutinizer:
	cd tests && cargo clean && cargo pear-scrutinizer

test-pear-filter:
	cd tests && cargo clean && cargo pear --filter $(FILTER)

test-scrutinizer-filter:
	cd tests && cargo clean && cargo pear-scrutinizer --filter $(FILTER)

test: test-pear test-scrutinizer

clean-pear:
	cargo clean

clean-tests:
	cd tests && cargo clean

clean-output:
	cd tests && rm -rf *.pear.* 
	cd tests/bodies && rm -rf *.mir.*

clean: clean-pear clean-tests clean-output
