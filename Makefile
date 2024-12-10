fetch-deps:
	git submodule update --init

install:
	cd peirce_frontend && cargo install --locked --path . 

test:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo peirce

test-filter:
	cd tests && cargo clean && RUST_BACKTRACE=full cargo peirce --filter $(FILTER)

clean-peirce:
	cargo clean

clean-tests:
	cd tests && cargo clean

clean-output:
	cd tests && rm -rf *.peirce.*

clean: clean-peirce clean-tests clean-output
