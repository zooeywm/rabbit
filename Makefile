SHELL := /bin/sh

CARGO ?= cargo
SLINT_LSP ?= slint-lsp
SUDO ?= sudo

RELEASE_BIN ?= target/release/rabbit
RABBIT_CAPS ?= cap_sys_admin+ep
ARGS ?=
RUN_ARGS ?= $(ARGS)

.DEFAULT_GOAL := help

.PHONY: help
help:
	@printf '%s\n' 'Rabbit developer targets:'
	@printf '%s\n' '  make run-release [RUN_ARGS="..."]   Build release, sudo setcap, then cargo run -r'
	@printf '%s\n' '  make setcap-release                 sudo setcap $(RABBIT_CAPS) $(RELEASE_BIN)'
	@printf '%s\n' '  make clearcap-release               Remove capabilities from $(RELEASE_BIN)'
	@printf '%s\n' '  make format-slint                   Format every ui/**/*.slint file with slint-lsp'
	@printf '%s\n' '  make test-gpu                       Run scripts/test-gpu'
	@printf '%s\n' '  make test-kms                       Run scripts/test-kms'
	@printf '%s\n' '  make test-gstreamer                 Run scripts/test-gstreamer'
	@printf '%s\n' '  make test-host-video [ARGS="..."]   Run scripts/test-host-video'
	@printf '%s\n' '  make test-client-video [ARGS="..."] Run scripts/test-client-video'
	@printf '%s\n' ''
	@printf '%s\n' 'Overrides:'
	@printf '%s\n' '  RABBIT_CAPS="cap_sys_admin+ep" RELEASE_BIN="target/release/rabbit"'

.PHONY: build-release
build-release:
	$(CARGO) build -r

.PHONY: setcap-release
setcap-release: build-release
	$(SUDO) setcap $(RABBIT_CAPS) $(RELEASE_BIN)
	@getcap $(RELEASE_BIN) || true

.PHONY: clearcap-release
clearcap-release:
	@if [ -e '$(RELEASE_BIN)' ]; then \
		$(SUDO) setcap -r $(RELEASE_BIN) >/dev/null 2>&1 || true; \
		getcap $(RELEASE_BIN) || true; \
	else \
		printf '%s\n' '$(RELEASE_BIN) does not exist'; \
	fi

.PHONY: run-release
run-release: setcap-release
	$(CARGO) run -r -- $(RUN_ARGS)

.PHONY: format-slint slint-format
format-slint slint-format:
	@command -v $(SLINT_LSP) >/dev/null 2>&1 || { \
		printf '%s\n' 'slint-lsp was not found in PATH' >&2; \
		exit 127; \
	}
	@find ui -type f -name '*.slint' -print0 | sort -z | xargs -0 -r $(SLINT_LSP) format -i

.PHONY: test-gpu test-kms test-gstreamer test-host-video test-client-video

test-gpu:
	./scripts/test-gpu $(ARGS)

test-kms:
	./scripts/test-kms $(ARGS)

test-gstreamer:
	./scripts/test-gstreamer $(ARGS)

test-host-video:
	./scripts/test-host-video $(ARGS)

test-client-video:
	./scripts/test-client-video $(ARGS)
