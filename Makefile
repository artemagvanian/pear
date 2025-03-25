fetch-deps:
	git submodule update --init

install:
	cd pear_frontend && cargo install --locked --path . 

test:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo pear

test-scrutinizer:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo pear-scrutinizer

test-filter:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo pear --filter $(FILTER)

test-scrutinizer-filter:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo pear-scrutinizer --filter $(FILTER)

clean-pear:
	cargo clean

clean-tests:
	cd tests && cargo clean

clean-output:
	cd tests && rm -rf *.pear.*

clean: clean-pear clean-tests clean-output
