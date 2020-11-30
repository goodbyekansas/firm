.PHONY: clean

# this makes sure that we can run the makefile standalone
target/wasm32-wasi/debug/libwasi_python_shims.a: src/lib.rs
	cargo build

tests/test.o: tests/test.c wasi_python_shims.h
	$(CC) -I . -c tests/test.c -o tests/test.o

test: target/wasm32-wasi/debug/libwasi_python_shims.a tests/test.o
	$(CC) -lwasi_python_shims -L target/wasm32-wasi/debug tests/test.o -o test

check: test
	wasmtime run --disable-cache ./test

clean:
	rm -f ./test
	rm -f ./tests/test.o
