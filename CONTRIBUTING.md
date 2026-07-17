# Contributing to Rabbit

Thank you for contributing to Rabbit. The project values small, reviewable, and verifiable changes.

## Development Principles

- Keep each change focused on one clearly defined problem.
- Do not mix unrelated refactoring or formatting with a functional change.
- Keep platform implementations in `infra`, business capabilities and core data types in `kernel`, and workflow orchestration in `app`.

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

If a verification step cannot be run, explain why in the handoff.

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
