.PHONY: all install check

testsrcs = tests/test.cpp

objs_root=objs
ifeq ($(BUILD_TARGET),host)
  check_command=
  obj_path=$(objs_root)/host
else
  check_command=wasmtime run --disable-cache --mapdir ::$$(mktemp -d -t test-temp-XXX)
  obj_path=$(objs_root)/wasi
endif

testobjs = $(patsubst %.cpp,$(obj_path)/%.o, $(testsrcs))

CXXFLAGS = -DTESTING -std=c++14 -g
LDFLAGS = -lfirm_function

prefix ?= /usr/local

all: test

$(obj_path)/%.o: %.cpp src/function.hh
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -I ./src -c $< -o $@

test: $(testobjs)
	$(CXX) $(LDFLAGS) $(testobjs) -o test

check: test
	$(check_command) ./test

host-check:
	BUILD_TARGET="host" CXX=$(CXX_FOR_BUILD) LD_LIBRARY_PATH=$$LD_LIBRARY_PATH:$$LIBSTDCXX_FOR_BUILD/lib $(MAKE) check

ifeq ($(BUILD_TARGET), host)
valgrind: test
	LD_LIBRARY_PATH=$$LD_LIBRARY_PATH:$$LIBSTDCXX_FOR_BUILD/lib valgrind --fullpath-after=$(PWD)/ --error-exitcode=1 --leak-check=full --show-leak-kinds=all ./test
else
valgrind:
	BUILD_TARGET="host" \
	CXX=$(CXX_FOR_BUILD) \
	$(MAKE) valgrind
endif

install:
	install -d $(prefix)/include/firm
	install -m 644 ./src/function.hh $(prefix)/include/firm/function.hh

clean:
	rm -f ./test
	rm -rf $(objs_root)
