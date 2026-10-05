# Episteme Agent Guide

Episteme is a private, local-first Rust application for document ingestion,
knowledge intelligence, and publication workflows.

## Constraints

- Private content and model requests stay on this Mac.
- Model endpoints must use explicit loopback IP addresses.
- External tools are invoked with argument arrays, never through a shell.
- The Obsidian vault is canonical; DuckDB contains rebuildable derived state.
- Tests use temporary vaults and databases. Never target the real vault.
- No operation may delete, publish, or overwrite user content automatically.

## Quality gates

```text
baml-cli generate
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features
cargo test --doc
```

`just ci` runs exactly this sequence. `just generate` regenerates `src/baml_client/`.

Run `cargo check` and `cargo clippy` after any Rust change, and the test suite whenever test files
change. Do not act on rust-analyzer or IDE diagnostics unless asked; they are frequently stale.

## Release

Releases run through cargo-rail, orchestrated by the `Cruxfile`. Read
[docs/release-runbook.md](docs/release-runbook.md) before touching the pipeline.

```text
crux run --strict --plugins ~/.agents/skills/rust-release-orchestrator/scripts/plugins.toml \
  --target release-plan Cruxfile
```

Non-obvious constraints, each of which has already caused a real failure:

- **`CHANGELOG.md` is hand-authored.** `.config/rail.toml` sets
  `crates.episteme-local.changelog.skip = true` because cargo-rail derives entries from commit
  subjects, and this repository's feature merges are typed `chore`. Write the `## Unreleased`
  entries yourself; `scripts/promote-changelog.sh` only retitles the heading.
- **Never blind-retry `release-apply`.** It bumps, commits, pushes, uploads, and tags in one
  non-idempotent sequence, so a retry advances the version. A single failed publish with a retry
  policy produced 0.2.0, 0.3.0, and 0.4.0, undone by the `98d6cfe` revert. Recover with
  `release-resume`.
- **Requires cargo-rail >= 0.25.0.** Under 0.17.3, `release-resume` could not upload at all: with
  `publication: in_progress`, `reconcile_publications` took the `wait_for_registry` branch and
  never called `publish_crate`, then falsely reported `was published but did not become
observable`. That is fixed — `publish_crate` is now called unconditionally. Do not downgrade.
- **The crates.io token must be exported as `CARGO_REGISTRY_TOKEN`.** `op plugin run -- cargo` does
  not work — it writes the token under `[registries.cratebox]`, not `[registry]`.
- **cargo-rail now pushes git itself.** `remote_effects = "push"`, because 0.25.0 refuses
  `--publish` while `remote_effects = "none"`. The old `push = false` still parses but is
  deprecated: it resolves to `remote_effects = "none"`, which is exactly the state `--publish`
  rejects. So the two are mutually exclusive rather than the old option being gone — mixing
  `remote_effects` with any of `push`, `create_github_release`, or `forge` is a hard parse error.
  The release commit reaches origin **before** the crates.io upload, and the tag is pushed after.
  A failed upload therefore leaves git ahead of the registry; reconcile with `release-resume`,
  never force-push. Both `release-plan` and `release-apply` need `--publish`, or the release will
  bump, commit, and tag a version it never uploads.

## Documentation

Grounded in source, not intent. When behaviour and documentation disagree, the code wins and the
doc is wrong.

| Document                                           | Covers                                     |
| -------------------------------------------------- | ------------------------------------------ |
| [README.md](README.md)                             | Install, configure, CLI usage              |
| [docs/architecture.md](docs/architecture.md)       | Structure, data flow, trust boundary       |
| [docs/api-reference.md](docs/api-reference.md)     | Public API, ports, domain types, CLI flags |
| [docs/release-runbook.md](docs/release-runbook.md) | Release procedure and failure recovery     |
| [docs/designs/](docs/designs/)                     | Per-feature design rationale               |

## Rust conventions

- Rust 2024 edition.
- `anyhow` at the CLI boundary; explicit `thiserror` errors in library code.
- No `unwrap()` or `expect()` in production code.
- Generated `src/baml_client/` code is exempt from hand-written lint policy and must never be
  edited directly; regenerate it from `baml_src/`.
- Keep external integrations behind narrow traits.
- Keep `main.rs` thin and test behavior through the library.
