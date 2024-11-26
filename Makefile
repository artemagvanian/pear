deps:
	git submodule update --init

install:
	cd peirce_frontend && cargo install --locked --path . 

test:
	cd tests && cargo clean && cargo peirce
