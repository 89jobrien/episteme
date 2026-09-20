---
version: 1
last_updated: 2026-09-19
next_review: 2026-09-26
---

# Recurring Mistakes Ledger

## Process Errors

### Sequential verification commands separated with semicolons

- **Occurrences**: 2
- **Date**: 2026-09-18
- **Affected**: Episteme TDD verification runs
- **Prevention**: Run each gate in a separate tool call during development and use `just ci` for
  the final fail-fast sequence.
- **Notes**: A Clippy failure did not provide a reliable guarantee that later semicolon-separated
  commands were skipped, weakening the evidence chain.

### Commit created directly on main

- **Occurrences**: 1
- **Date**: 2026-09-18 (`093f5dc`)
- **Affected**: Initial Episteme root commit
- **Prevention**: Run `git branch --show-current` immediately before every commit and stop when it
  returns `main`; create and switch to a feature branch before staging or committing.
- **Notes**: The branch guard rule was supplied after the root commit, but it is now an explicit
  session requirement and must be enforced on all future commits.

### Assumed local model port without checking ownership

- **Occurrences**: 1
- **Date**: 2026-09-18
- **Affected**: Live Apfel verification
- **Prevention**: Probe the chosen port and inspect the server startup log before sending test
  traffic. Reserve `18181` for scoped Episteme Apfel tests and never use Ollama's `11434` port.
- **Notes**: Apfel failed to bind while the readiness probe reached the existing Ollama service.

### Release bookkeeping dirtied the package being published

- **Occurrences**: 1
- **Date**: 2026-09-19
- **Affected**: First real `episteme-local` Crux publish attempt
- **Prevention**: Store resumable release state under ignored build state such as
  `target/.release-state.json`, never at the workspace root. Add a release-runner regression test
  before changing state placement.
- **Notes**: Writing `.release-state.json` at the workspace root caused `cargo publish` to reject the
  otherwise clean package. The runner and skill documentation now use the target directory.

### Release dry-run unnecessarily required interactive credentials

- **Occurrences**: 1
- **Date**: 2026-09-19
- **Affected**: First publishable-package Crux dry-run
- **Prevention**: Invoke `cargo publish --dry-run` directly; reserve the 1Password-wrapped Cargo
  invocation for real uploads. Keep the direct-program assertion in runner integration tests.
- **Notes**: The non-interactive 1Password shell plugin timed out even though dry-run does not need a
  registry token.

### Crates.io package-name ownership checked late

- **Occurrences**: 1
- **Date**: 2026-09-19
- **Affected**: Initial plan to publish the package as `episteme`
- **Prevention**: Run an exact crates.io ownership check before adding publication metadata or
  choosing release tags. Decide package, library, and binary names independently when occupied.
- **Notes**: The existing `episteme` crate is unrelated; publication proceeded as `episteme-local`
  while retaining `episteme` for the library and executable.

## Test Failures

### Sparse live fixture failed strict BAML schema validation

- **Occurrences**: 1
- **Date**: 2026-09-18
- **Affected**: `live_ingestion_uses_temporary_vault`
- **Prevention**: Live extraction fixtures must include explicit title, author, citation, summary,
  critique, and verbatim evidence suitable for deterministic grounding checks.
- **Notes**: The richer fixture passed on the second live attempt.

### Typed model output mistaken for grounded evidence

- **Occurrences**: 3
- **Date**: 2026-09-19
- **Affected**: Chunked live ingestion with `llama3.2` and `gpt-mbx`
- **Prevention**: Let models select stable source-span IDs, validate those IDs against map outputs,
  and reconstruct quote text and locations deterministically in Rust. Never treat schema-valid
  strings as proof of semantic grounding.
- **Notes**: Both models returned valid BAML structures but paraphrased evidence. Strict substring
  validation correctly rejected all three attempts before the span-ID redesign passed live tests.

## Clippy Lints

- Nothing notable.

## Hook False Positives

- Nothing notable.

## Reverts

- Nothing notable.
