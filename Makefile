fetch-deps:
	git submodule update --init

install:
	cd pear_frontend && cargo install --locked --path . 

test:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo pear

test-filter:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo pear --filter $(FILTER)

clean-pear:
	cargo clean

clean-tests:
	cd tests && cargo clean

clean-output:
	cd tests && rm -rf *.pear.*

clean: clean-pear clean-tests clean-output
