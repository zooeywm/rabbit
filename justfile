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

run-xwin: (_xwin "run")
run-xwin-testui: (_xwin "run" "--features" "test-ui" "--" "--test")
build-xwin: (_xwin "build")
check-xwin: (_xwin "check" "--all-targets" "--all-features")
lint-xwin: (_xwin "clippy" "--all-targets" "--all-features")

check:
    cargo check --all-targets --all-features
lint:
    cargo clippy --all-targets --all-features
run:
    cargo run
run-testui:
    cargo run --features test-ui -- --test
run-fake-testui:
    cargo run --features fake,test-ui -- --test
run-fake:
    cargo run --features fake
build:
    cargo build
build-testui:
    cargo build --features test-ui
build-fake-testui:
    cargo build --features fake,test-ui
build-fake:
    cargo build --features fake
test:
    cargo nextest run
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
