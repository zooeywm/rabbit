SHELL := /bin/sh

CARGO ?= cargo
CARGO_XWIN ?= cargo xwin
SLINT_LSP ?= slint-lsp
SUDO ?= sudo

HOST_STRIP := $(firstword $(foreach tool,strip llvm-strip,$(shell command -v $(tool) 2>/dev/null)))

RELEASE_BIN ?= target/release/rabbit
DEBUG_BIN ?= target/debug/rabbit
RABBIT_CAPS ?= cap_sys_admin+ep

ARGS ?=
RUN_ARGS ?= $(ARGS)

# Extra args after `record`
# Example:
#   make record RECORD_ARGS="-s HDMI-A-1 -d 30"
RECORD_ARGS ?=

# Cross-compile target for cargo-xwin
XWIN_TARGET ?= x86_64-pc-windows-msvc

# Optional: RABBIT_KMS_SCREEN for host-video / record hardware paths
# RABBIT_KMS_SCREEN ?= HDMI-A-1


# -----------------------------------------------------------------------------
# Build configuration
# -----------------------------------------------------------------------------

# Supported values:
#   PROFILE=debug
#   PROFILE=release
PROFILE ?= debug

# Supported values:
#   STRIP=0
#   STRIP=1
STRIP ?= 0

ifeq ($(filter $(PROFILE),debug release),)
$(error PROFILE must be either debug or release)
endif

ifeq ($(filter $(STRIP),0 1),)
$(error STRIP must be either 0 or 1)
endif

CARGO_PROFILE_ARG := $(if $(filter release,$(PROFILE)),--release,)
RABBIT_BIN := $(if $(filter release,$(PROFILE)),$(RELEASE_BIN),$(DEBUG_BIN))
STRIP_REQUESTED := $(filter 1,$(STRIP))


# -----------------------------------------------------------------------------
# Strip helper
# -----------------------------------------------------------------------------

define atomic_strip
@strip_source='$(2)'; \
strip_temp="$$(mktemp "$${strip_source}.strip.XXXXXX")" || exit 1; \
trap 'rm -f -- "$$strip_temp"' 0 1 2 15; \
$(1) --strip-all -o "$$strip_temp" "$$strip_source"; \
chmod --reference="$$strip_source" "$$strip_temp"; \
mv -f -- "$$strip_temp" "$$strip_source"; \
trap - 0 1 2 15
endef


# -----------------------------------------------------------------------------
# Help
# -----------------------------------------------------------------------------

.DEFAULT_GOAL := help

.PHONY: help

help:
	@printf '%s\n' 'Rabbit developer targets:'
	@printf '%s\n' '  make build                         Build debug Rabbit'
	@printf '%s\n' '  make build PROFILE=release         Build release Rabbit'
	@printf '%s\n' '  make build PROFILE=release STRIP=1 Build and strip release Rabbit'
	@printf '%s\n' ''
	@printf '%s\n' '  make run                           Build, setcap, and run Rabbit'
	@printf '%s\n' '  make setcap                        Build and set capability'
	@printf '%s\n' '  make clearcap                      Remove capability'
	@printf '%s\n' '  make record                        Build, setcap, and record'
	@printf '%s\n' ''
	@printf '%s\n' '  make format-slint                  Format every ui/**/*.slint file'
	@printf '%s\n' '  make test-gpu                      Run scripts/test-gpu'
	@printf '%s\n' '  make test-kms                      Run scripts/test-kms'
	@printf '%s\n' '  make test-gstreamer                Run scripts/test-gstreamer'
	@printf '%s\n' '  make test-host-video               Run scripts/test-host-video'
	@printf '%s\n' ''
	@printf '%s\n' '  make xwin-build                    Build Windows executable'
	@printf '%s\n' '  make xwin-check                    Check Windows target'
	@printf '%s\n' '  make xwin-test                     Test Windows target'
	@printf '%s\n' ''
	@printf '%s\n' 'Overrides:'
	@printf '%s\n' '  PROFILE=debug|release'
	@printf '%s\n' '  STRIP=0|1'
	@printf '%s\n' '  RABBIT_CAPS'
	@printf '%s\n' '  DEBUG_BIN'
	@printf '%s\n' '  RELEASE_BIN'
	@printf '%s\n' '  XWIN_TARGET'
	@printf '%s\n' '  ARGS'
	@printf '%s\n' '  RUN_ARGS'
	@printf '%s\n' '  RECORD_ARGS'
	@printf '%s\n' ''
	@printf '%s\n' 'Examples:'
	@printf '%s\n' '  make run'
	@printf '%s\n' '  make run PROFILE=release'
	@printf '%s\n' '  make run PROFILE=release STRIP=1 RUN_ARGS="--help"'
	@printf '%s\n' '  make record RECORD_ARGS="-s HDMI-A-1 -d 30"'
	@printf '%s\n' '  make xwin-build PROFILE=release'


# -----------------------------------------------------------------------------
# Linux build
# -----------------------------------------------------------------------------

.PHONY: build

build:
	$(CARGO) build $(CARGO_PROFILE_ARG)
ifneq ($(STRIP_REQUESTED),)
	@[ -n '$(HOST_STRIP)' ] || { \
		printf '%s\n' \
			'No ELF strip tool was found (tried strip and llvm-strip)' >&2; \
		exit 127; \
	}
	$(call atomic_strip,$(HOST_STRIP),$(RABBIT_BIN))
endif


# -----------------------------------------------------------------------------
# Linux capabilities
# -----------------------------------------------------------------------------

.PHONY: setcap clearcap

setcap: build
	$(SUDO) setcap $(RABBIT_CAPS) $(RABBIT_BIN)
	@getcap $(RABBIT_BIN) || true

clearcap:
	@if [ -e '$(RABBIT_BIN)' ]; then \
		$(SUDO) setcap -r '$(RABBIT_BIN)' >/dev/null 2>&1 || true; \
		getcap '$(RABBIT_BIN)' || true; \
	else \
		printf '%s\n' '$(RABBIT_BIN) does not exist'; \
	fi


# -----------------------------------------------------------------------------
# Run
# -----------------------------------------------------------------------------

.PHONY: run record

run: setcap
	$(CARGO) run $(CARGO_PROFILE_ARG) -- $(RUN_ARGS)

# Local screen recording.
# KMS paths require cap_sys_admin, same as run.
record: setcap
	$(CARGO) run $(CARGO_PROFILE_ARG) -- record $(RECORD_ARGS)


# -----------------------------------------------------------------------------
# Slint
# -----------------------------------------------------------------------------

.PHONY: format-slint slint-format

format-slint slint-format:
	@command -v '$(SLINT_LSP)' >/dev/null 2>&1 || { \
		printf '%s\n' 'slint-lsp was not found in PATH' >&2; \
		exit 127; \
	}
	@find ui \
		-type f \
		-name '*.slint' \
		-print0 \
		| sort -z \
		| xargs -0 -r '$(SLINT_LSP)' format -i


# -----------------------------------------------------------------------------
# Test scripts
# -----------------------------------------------------------------------------

.PHONY: \
	test-gpu \
	test-kms \
	test-gstreamer \
	test-host-video

test-gpu:
	./scripts/test-gpu $(ARGS)

test-kms:
	./scripts/test-kms $(ARGS)

test-gstreamer:
	./scripts/test-gstreamer $(ARGS)

test-host-video:
	./scripts/test-host-video $(ARGS)


# -----------------------------------------------------------------------------
# Windows cross-compilation via cargo-xwin
# -----------------------------------------------------------------------------

.PHONY: xwin-build xwin-check xwin-test

xwin-build:
	@command -v cargo-xwin >/dev/null 2>&1 || { \
		printf '%s\n' \
			'cargo-xwin not found; install with: cargo install cargo-xwin' >&2; \
		exit 127; \
	}
	$(CARGO_XWIN) build \
		$(CARGO_PROFILE_ARG) \
		--target $(XWIN_TARGET)

xwin-check:
	@command -v cargo-xwin >/dev/null 2>&1 || { \
		printf '%s\n' \
			'cargo-xwin not found; install with: cargo install cargo-xwin' >&2; \
		exit 127; \
	}
	$(CARGO_XWIN) check \
		$(CARGO_PROFILE_ARG) \
		--target $(XWIN_TARGET)

xwin-test:
	@command -v cargo-xwin >/dev/null 2>&1 || { \
		printf '%s\n' \
			'cargo-xwin not found; install with: cargo install cargo-xwin' >&2; \
		exit 127; \
	}
	$(CARGO_XWIN) test \
		$(CARGO_PROFILE_ARG) \
		--target $(XWIN_TARGET)
