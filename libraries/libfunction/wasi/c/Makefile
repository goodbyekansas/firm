.PHONY: clean default check check-debug rust valgrind

default: test

headers = $(shell find src/ -type f -name '*.h') tests/test.h
testsrcs = \
	tests/test.c \
	tests/mock.c \
	tests/common.c \
	tests/string.c \
	tests/integer.c \
	tests/float.c \
	tests/boolean.c \
	tests/bytes.c

CFLAGS = -Werror -g -std=c99
LDFLAGS = -lpthread -ldl -lm

objs_root=objs
ifeq ($(BUILD_TARGET),host)
  check_command=
  obj_path=$(objs_root)/host
else
  check_command=wasmtime run --disable-cache
  obj_path=$(objs_root)/wasi
endif

test_objs = $(patsubst %.c,$(obj_path)/%.o, $(testsrcs))

# this makes sure that we can run the makefile standalone
rust:
	RUSTFLAGS="--cfg=host_test $(RUSTFLAGS)" cargo build --release

$(obj_path)/%.o: %.c $(headers)
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -I ./src -c $< -o $@

test: rust $(test_objs)
	@echo $(test_file_names)
	@echo $(test_dirs)
	@echo $(test_objs)
	$(CC) $(LDFLAGS) $(test_objs) target/$(CARGO_BUILD_TARGET)/release/libfirm_function.a -o test

check: test
	$(check_command) ./test

ifeq ($(BUILD_TARGET), host)
valgrind: test
	valgrind --fullpath-after=$(PWD)/ --error-exitcode=1 --leak-check=full --show-leak-kinds=all ./test
else
valgrind:
	BUILD_TARGET="host" \
	CC=$(CC_FOR_BUILD) \
	CARGO_BUILD_TARGET=$(shell $(CC_FOR_BUILD) -dumpmachine) $(MAKE) valgrind
endif

host-check:
	BUILD_TARGET="host" \
	CC=$(CC_FOR_BUILD) \
	CARGO_BUILD_TARGET=$(shell $(CC_FOR_BUILD) -dumpmachine) $(MAKE) check

check-debug: test
	lldb \
	-O 'settings set plugin.jit-loader.gdb.enable on' \
	-O "command regex pp 's/(.+)/p __vmctx->set(),%1/'" \
	-- wasmtime run -g --opt-level 0 ./test

profile: test
	perf record -k mono wasmtime --disable-cache --jitdump ./test
	perf inject --jit --input perf.data --output perf.jit.data
	perf report --input perf.jit.data
	rm -f perf.data perf.jit.data jitted-*.so jit-*.dump

clean:
	rm -f ./test
	rm -rf $(objs_root)
