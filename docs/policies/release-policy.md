# Release Policy

## Versioning

BetCode uses [Semantic Versioning](https://semver.org/) with relaxed pre-1.0
semantics:

- **Patch** (0.1.0 → 0.1.1): Bug fixes, documentation, internal refactors
- **Minor** (0.1.x → 0.2.0): New features, non-breaking additions, and
  breaking changes (allowed pre-1.0 per SemVer spec)
- **Major** (0.x → 1.0): Reserved for production-ready stability commitment

All workspace crates share a single version number defined in the root
`Cargo.toml` under `[workspace.package]`.

## Release Cadence

Releases are **feature-based**, not time-based. A release happens when a
meaningful set of features or fixes is ready. There is no fixed schedule.

## Proto Compatibility

The `betcode-proto` submodule defines the gRPC API contract shared between
the backend and the mobile app. Proto changes drive the versioning strategy:

| Change type | Examples | Version impact | Coordination |
|-------------|----------|----------------|--------------|
| Non-breaking addition | New field, new RPC, new message | Any release | None required |
| Breaking change | Field removal, type change, removed RPC | Minor bump required | Mobile app must update first |

### Breaking Proto Changes

1. Open an issue to discuss the change and its impact
2. Update the mobile app repo to handle both old and new formats
3. Release a mobile app version that supports the new format
4. Merge the backend proto change with a `BREAKING CHANGE` commit footer
5. Tag the backend release

This ensures users are never stuck with an incompatible app/backend pair.

## Release Process

### Tooling

We use [release-plz](https://release-plz.ieni.dev/) to automate changelog
generation and version bumping.

### Steps

1. **Maintainer decides** a set of changes is ready for release
2. **Run `release-plz`** which:
   - Reads conventional commits since the last tag
   - Proposes a version bump based on commit types
   - Generates a `CHANGELOG.md` entry
   - Opens a release PR
3. **Review the PR**: adjust changelog prose if needed, verify version bump
   is appropriate
4. **Merge the PR**: this updates `Cargo.toml` versions and `CHANGELOG.md`
5. **Tag the release**: `release-plz` creates the `v*` tag automatically
6. **CI publishes**: the `release.yml` workflow triggers, building binaries
   for all platforms and publishing a GitHub Release

### Artifacts

Each release produces platform-specific archives:

| Platform | Archive | Binaries |
|----------|---------|----------|
| Linux x86_64 | `.tar.gz` | betcode, betcode-daemon, betcode-relay, betcode-setup, betcode-releases |
| Linux ARM64 | `.tar.gz` | betcode, betcode-daemon, betcode-relay, betcode-setup, betcode-releases |
| macOS x86_64 | `.tar.gz` | betcode, betcode-daemon |
| macOS ARM64 | `.tar.gz` | betcode, betcode-daemon |
| Windows x86_64 | `.zip` | betcode, betcode-daemon |

## Changelog

The `CHANGELOG.md` file follows the
[Keep a Changelog](https://keepachangelog.com/) format. It is auto-generated
from conventional commit messages by `release-plz` and can be edited by the
maintainer before the release PR is merged.

Sections used:

- **Added** — from `feat` commits
- **Fixed** — from `fix` commits
- **Changed** — from `refactor` commits with visible behavior changes
- **Breaking** — from commits with `BREAKING CHANGE` footer

## Hotfix Process

For critical bug fixes that need immediate release:

1. Create a branch from the latest release tag
2. Apply the minimal fix with tests
3. Follow the normal release process (release-plz PR → review → merge → tag)
4. Cherry-pick the fix back to `master` if not already there
