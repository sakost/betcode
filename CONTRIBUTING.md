# Contributing to BetCode

Thank you for your interest in contributing to BetCode! This guide covers
everything you need to get started.

## Getting Started

### Prerequisites

- **Rust** stable toolchain (install via [rustup](https://rustup.rs/))
- **protoc** (protobuf compiler) — `apt install protobuf-compiler` on Ubuntu,
  `brew install protobuf` on macOS
- **just** (command runner) — `cargo install just` or see
  [just installation](https://github.com/casey/just#installation)
- **Node.js 20+** — for duplicate code detection (`jscpd`)

### Setup

```bash
git clone --recurse-submodules https://github.com/sakost/betcode.git
cd betcode
cargo build
just check  # runs all quality gates — if this passes, CI will pass
```

If you forgot `--recurse-submodules`:

```bash
git submodule update --init --recursive
```

## Making Changes

### Workflow

1. Fork the repository and clone your fork with `--recurse-submodules`
2. Create a feature branch from `master` (`git checkout -b feat/my-feature`)
3. Make your changes, adding tests for new features
4. Run `just check` to verify everything passes
5. Commit using [conventional commit](#commit-messages) messages
6. Push your branch and open a pull request

### Useful Commands

| Command | Description |
|---------|-------------|
| `just check` | Run all quality gates (what CI runs) |
| `just fmt` | Auto-format code |
| `just lint` | Run Clippy with strict warnings |
| `just test` | Run the full test suite |
| `just deny` | Check dependency licenses and advisories |
| `just machete` | Detect unused dependencies |
| `just duplicates` | Detect duplicate code |
| `just fix` | Auto-fix Clippy lints where possible |
| `just build` | Full workspace build |

### Pre-commit Hooks

`cargo-husky` runs automatically on every commit:

- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt -- --check`

Do not bypass hooks with `--no-verify` unless you have a specific reason.
CI will catch anything hooks miss.

## Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/). Every
commit message must follow this format:

```
type(scope): short description

Optional longer body explaining context and reasoning.

BREAKING CHANGE: description of what breaks (if applicable)
```

### Types

| Type | When to use |
|------|-------------|
| `feat` | New feature or functionality |
| `fix` | Bug fix |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `test` | Adding or updating tests |
| `docs` | Documentation only |
| `ci` | CI/CD pipeline changes |
| `chore` | Maintenance tasks (deps, config, tooling) |
| `style` | Formatting, linting fixes (no logic change) |

### Scopes

Scopes are optional but encouraged. Use crate names or cross-cutting concerns:

`daemon`, `relay`, `cli`, `proto`, `setup`, `releases`, `crypto`, `core`,
`lint`, `deps`

### Breaking Changes

For breaking changes, add `!` after the type/scope and include a
`BREAKING CHANGE:` footer:

```
feat(proto)!: remove legacy session fields

Removed deprecated `old_session_id` and `legacy_token` fields from
SessionRequest message.

BREAKING CHANGE: Clients using old_session_id must migrate to session_id.
Mobile app v0.3+ is required.
```

## Pull Request Requirements

All of the following must be satisfied before a PR can be merged:

- **CI passes** — fmt, clippy, cargo-deny, machete, jscpd, and all tests
- **Tests included** — new features must have tests; bug fixes should add a
  regression test where practical
- **Conventional commits** — all commits in the PR follow the convention
- **PR title** — follows conventional commit format (used in changelog for
  squash merges)

### What We Look For

- Clean, focused changes — one concern per PR
- Tests that cover the new behavior
- No unnecessary dependency additions
- No `unwrap()` in production code
- Documentation for public APIs

### What We Won't Merge

- Proto contract changes without prior coordination (open an issue first)
- Features without tests
- PRs that don't pass CI
- Cosmetic-only refactors unless they measurably improve code quality

## Code Quality Standards

These are enforced by CI and cannot be bypassed:

- **No `unwrap()` in production code** (denied by Clippy)
- **No `panic!`, `todo!`, `dbg!` in production code**
- **Clippy pedantic + nursery** lints enabled
- **Code duplication** must stay under 0.07% (jscpd)
- **No unused dependencies** (cargo-machete)
- **All dependency licenses** must be in the allow-list (`deny.toml`)

For test code, `expect()`, `unwrap()`, and `panic!()` are fine — add a
granular `#[allow(clippy::...)]` on the specific test function.

## Adding Dependencies

1. Check that the license is in our allow-list (`deny.toml`)
2. Add workspace-level dependencies to the root `Cargo.toml` under
   `[workspace.dependencies]`
3. Reference from crate `Cargo.toml` with `dep = { workspace = true }`
4. Run `just deny` to verify license and advisory compliance
5. Update `THIRD-PARTY-LICENSES.md` if a new license type is introduced

## Proto Changes

The `proto` submodule contains the protobuf definitions shared between
the backend and the mobile app.

- **Non-breaking additions** (new fields, new RPCs, new messages): can ship
  in any release
- **Breaking changes** (field removals, type changes, removed RPCs):
  1. Open an issue to discuss the change
  2. Coordinate with the mobile app repository
  3. Use the `BREAKING CHANGE` commit footer
  4. Both repos must be updated before either is released

To regenerate after editing `.proto` files:

```bash
cargo build -p betcode-proto
```

See also: [Release Policy](docs/policies/release-policy.md) for versioning rules and the breaking change release process.

## Reporting Issues

- **Bug reports**: use the Bug Report issue template
- **Feature requests**: use the Feature Request template
- **Security issues**: email the maintainer directly (do not open a public issue)

Look for issues labeled `good first issue` if you're new, or `help wanted`
for more substantial tasks.

## Review Process

All PRs are reviewed by a maintainer. Expect feedback within a few days.
Small fixes may be merged quickly; larger features may need iteration.

If your PR sits without review for more than a week, feel free to ping.

## License

By contributing to BetCode, you agree that your contributions will be
licensed under the [Apache-2.0 license](LICENSE).
