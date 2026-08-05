set dotenv-load

windows-target := "x86_64-pc-windows-msvc"

default:
    @just --list

[private]
_cargo command *args:
    cargo {{ command }} {{ args }}

[private]
_xwin command *args:
    cargo xwin {{ command }} --target {{ windows-target }} {{ args }}

run: (_cargo "run")
run_fake: (_cargo "run" "--features" "fake")
build: (_cargo "build" "--workspace")
check: (_cargo "check" "--workspace" "--all-targets")
lint: (_cargo "clippy" "--workspace" "--all-targets" "--all-features" "--" "-D" "warnings")
release: (_cargo "build" "--workspace" "--release")

run-xwin: (_xwin "run")
run_fake-xwin: (_xwin "run" "--features" "fake")
build-xwin: (_xwin "build" "--workspace")
check-xwin: (_xwin "check" "--workspace" "--all-targets")
lint-xwin: (_xwin "clippy" "--workspace" "--all-targets" "--all-features" "--" "-D" "warnings")
release-xwin: (_xwin "build" "--workspace" "--release")

test:
    cargo nextest run --workspace
fmt:
    cargo fmt --all
fmt-check:
    cargo fmt --all -- --check
dev:
    RUST_LOG=info,rabbit=debug cargo run
trace:
    RUST_LOG=trace cargo run
clean:
    cargo clean
