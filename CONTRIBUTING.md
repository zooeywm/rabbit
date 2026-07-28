<!-- lang: en (default) -->
> **Language / 语言:** **English** · [中文](./CONTRIBUTING.zh.md)

# Contributing to Rabbit

Thank you for contributing to Rabbit. The project values small, reviewable, and verifiable changes.

## Development Principles

- Keep each change focused on one clearly defined problem.
- Do not mix unrelated refactoring or formatting with a functional change.
- Keep platform implementations in `infra`, business capabilities and core data types in `kernel`, and workflow orchestration in `app`.
- Do not suppress Rust's `dead_code` lint; remove unused code, exercise it in production, or place it behind a real module boundary.
- Keep platform conditionals inside platform implementations, except for the direct `mod platform` path selection in `app/mod.rs` and `infra/mod.rs`. Generic modules must use a uniform platform interface.
- Modules with child modules must use the `name/mod.rs` layout; do not combine `name.rs` with a sibling `name/` directory. The `app::platform` and `infra::platform` aliases are deliberately mapped directly to `platform/linux/mod.rs` or `platform/windows/mod.rs`.

## Architecture

The system architecture, dependency rules, media paths, extension playbooks, and
invariants live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Read that document before
changing session protocol, layer boundaries, or platform stacks.

### Layer map (summary)

| Layer | Path | Owns |
| --- | --- | --- |
| `kernel` | `src/kernel/` | Capability traits, session/control protocol, core value types |
| `infra` | `src/infra/` | QUIC/TCP, platform capture/encode/decode/render |
| `app` | `src/app/` | Config, services, GUI workflow, stack assembly |
| UI | `ui/` | Slint views bound through `app::gui::view` |

Prefer landing media-path changes on the Linux stack first unless the change is Windows-specific.

## Workflow

1. State the assumptions, scope, and acceptance criteria before making changes.
2. Establish the focused test described below before implementing a new module.
3. Implement the smallest change that can be reviewed independently.
4. Run the focused test for the affected module.
5. Inspect the diff and remove unrelated changes.
6. Wait for approval before starting the next independent change.

## Test-First Module Development

Before implementing or integrating a new module:

1. Add the smallest compilable module boundary with a deterministic no-op or empty implementation.
2. Add a focused unit test alongside the module that imports its boundary and runs successfully.
3. Record the exact command that runs only that test target.
4. Replace the empty implementation incrementally while keeping the test executable and passing.

Every commit that changes the module must be preceded by a successful run of its focused test. Do not create the commit when that test fails or cannot run. Keep the test after the module is integrated into the application.

Keep tests for private implementation modules as co-located unit tests so they can exercise private boundaries without widening the production API. Use integration tests under `tests/` only when testing an intended public boundary or behavior spanning multiple modules.

## Rust Verification

Run the affected module's focused test before every commit. A compilation-only command is not a substitute for that test.

Choose other verification commands based on the risk of the change and the current review instructions:

```shell
cargo fmt --check
cargo check
```

For changes involving linting, concurrency, security boundaries, or public interfaces, run the following as appropriate:

```shell
cargo test
cargo clippy --all-targets
```

For Windows-specific changes, cross-check the Windows target with `cargo-xwin`:

```shell
cargo xwin clippy --target x86_64-pc-windows-msvc
```

If a verification step cannot be run, explain why in the handoff.

## Linux Remote Input Permission

Remote keyboard and pointer injection use `/dev/uinput`. Grant the user running
Rabbit read/write access with a udev rule instead of running Rabbit as root:

```shell
sudo groupadd -f uinput
sudo usermod -aG uinput "$USER"
echo 'KERNEL=="uinput", GROUP="uinput", MODE:="0660"' \
  | sudo tee /etc/udev/rules.d/99-rabbit-uinput.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=uinput
```

Log out and back in after changing group membership.

Pointer movement defaults to absolute positioning. To exercise reliable relative
movement, set the following in `config.toml`:

```toml
[input]
pointer_mode = "relative"
```

## Commit Convention

Commit subjects follow Conventional Commits:

```text
<type>(<scope>): <summary>
```

The `scope` is optional. Each commit should represent one change that can be explained, reviewed, and reverted independently.

### Type

| Type | Purpose |
| --- | --- |
| `feat` | Add a user-visible capability or a new system capability |
| `fix` | Fix a defect |
| `refactor` | Restructure code without changing external behavior |
| `perf` | Improve performance |
| `test` | Add or update tests |
| `docs` | Change documentation only |
| `build` | Change the build system or maintain dependencies |
| `ci` | Change CI configuration |
| `chore` | Perform maintenance that does not fit another type |
| `revert` | Revert an existing commit |

Use `feat(deps)` when a dependency introduces a new system capability. Use `build(deps)` when upgrading, downgrading, or maintaining an existing dependency.

### Scope

Prefer a lowercase scope that identifies the affected boundary:

- `app`: application lifecycle and workflow orchestration.
- `kernel`: capability interfaces and core data types.
- `infra`: platform or external-system implementations.
- `deps`: dependencies.
- `config`: configuration.
- `logging`: logging.
- `docs`: documentation structure spanning multiple documents.

Omit the scope when no clear scope exists. Do not invent an ambiguous scope only to fill the field.

### Summary

- Use an English imperative phrase beginning with a lowercase letter, such as `add compio runtime dependency`.
- Describe the completed result, not the steps taken.
- Do not end the summary with a period.
- Keep the subject within 72 characters when practical.

### Body and Footer

A simple change may use only a subject. Add a body after a blank line when the motivation, tradeoffs, or behavior are not obvious. Focus the body on why the change was made.

Mark a breaking change with `!` after the type or scope and describe it in the footer:

```text
feat(protocol)!: replace legacy handshake

BREAKING CHANGE: peers using the legacy handshake can no longer connect.
```

Use `Refs:` or `Closes:` in the footer when linking an issue.

### Examples

```text
feat(deps): add compio runtime dependency
feat(kernel): add screen capture subscription interface
fix(infra): preserve screen layout after refresh failure
docs: add contribution and commit guidelines
refactor(app): separate session creation from dependency assembly
```
