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
- **Never blind-retry `release-apply`.** It bumps, commits, tags, and uploads in one
  non-idempotent sequence, so a retry advances the version. A single failed publish with a retry
  policy produced 0.2.0, 0.3.0, and 0.4.0, undone by the `98d6cfe` revert.
- **`release-resume` re-attempts the observation, not the upload.** Verified against cargo-rail
  0.17.3: with `publication: in_progress`, `reconcile_publications` takes the `wait_for_registry`
  branch and never calls `publish_crate`, so an unuploaded version is never uploaded by that
  target. It then reports `was published but did not become observable`, which is false in both
  halves. Check the crates.io API and tarball first; if the version is absent, `cargo publish`
  directly, then `resume` to reconcile state.
- **The crates.io token must be exported as `CARGO_REGISTRY_TOKEN`.** `op plugin run -- cargo` does
  not work — it writes the token under `[registries.cratebox]`, not `[registry]`.
- **Check the installed cargo-rail version before trusting any of this.** The local binary was
  0.17.3 while upstream was 0.30.1; the resume behaviour above is specific to the installed
  version's `publisher.rs`.
- cargo-rail does not push. `push = false`, so `git push origin main` and the tag push are manual.

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
