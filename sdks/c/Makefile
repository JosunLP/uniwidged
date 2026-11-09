# Builds a uniwidged widget written in C.
#
#   make                         the example, into build/c-counter.wasm
#   make WIDGET=path/to/dir      your own: every .c file in that directory
#
# Needs two tools, and names whichever is missing:
#
#   wasi-sdk 25 or later         WASI_SDK_PATH=/opt/wasi-sdk (the default);
#                                its clang targets wasm32-wasip2 and links a
#                                component directly, through wasm-component-ld
#   wit-bindgen 0.61             cargo install wit-bindgen-cli --version 0.61.1
#
# The bindings are generated from the interface rather than committed: the
# interface is the contract, and a generated file that drifted from it would be
# a second contract nobody agreed to.

WASI_SDK_PATH ?= /opt/wasi-sdk
WIT_BINDGEN ?= wit-bindgen
CC := $(WASI_SDK_PATH)/bin/clang
WIT := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))../../wit/uniwidged.wit)
SDK := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))

WIDGET ?= $(SDK)/examples/counter
NAME ?= $(shell sed -n 's/^name = "\(.*\)"$$/\1/p' $(WIDGET)/widget.toml)
BUILD ?= $(SDK)/build
BINDINGS := $(BUILD)/bindings

CFLAGS ?= -O2
CFLAGS += --target=wasm32-wasip2 -std=c11 -Wall -Wextra -Werror \
          -mexec-model=reactor -I$(SDK)/include -I$(BINDINGS)

SOURCES := $(wildcard $(WIDGET)/*.c) $(SDK)/src/uniwidged.c $(BINDINGS)/widget.c

.PHONY: all clean check-tools
all: $(BUILD)/$(NAME).wasm

check-tools:
	@test -x "$(CC)" || { echo "no clang at $(CC): set WASI_SDK_PATH to a wasi-sdk (25 or later)" >&2; exit 1; }
	@command -v "$(WIT_BINDGEN)" >/dev/null || { echo "no $(WIT_BINDGEN): cargo install wit-bindgen-cli --version 0.61.1" >&2; exit 1; }

$(BINDINGS)/widget.c $(BINDINGS)/widget.h $(BINDINGS)/widget_component_type.o: $(WIT) | check-tools
	@mkdir -p $(BINDINGS)
	$(WIT_BINDGEN) c $(WIT) --world widget --out-dir $(BINDINGS)

# The generated C is the bindings' business, and its warnings are not this
# widget's to fix; the widget's own files and the SDK are held to -Werror.
$(BUILD)/$(NAME).wasm: $(SOURCES) $(SDK)/include/uniwidged.h $(BINDINGS)/widget_component_type.o | check-tools
	$(CC) $(CFLAGS) -Wno-unused-parameter $(SOURCES) $(BINDINGS)/widget_component_type.o -o $@
	@cp $(WIDGET)/widget.toml $(BUILD)/$(NAME).toml
	@echo "built $@"

clean:
	rm -rf $(BUILD)
