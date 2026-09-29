# Release runbook

How to cut a release of `episteme-local` to crates.io, and how to recover when a publish fails
partway. Written against cargo-rail 0.17.3 and the `Cruxfile` at the repository root.

## Contents

- [What owns what](#what-owns-what)
- [Gate order](#gate-order)
- [Why `--bump minor`](#why---bump-minor)
- [The changelog cut](#the-changelog-cut)
- [Credentials](#credentials)
- [The apply step is not idempotent](#the-apply-step-is-not-idempotent)
- [Verify the actual outcome — never trust the error message](#verify-the-actual-outcome--never-trust-the-error-message)
- [Recovery](#recovery)
  - [Pass the state file explicitly](#pass-the-state-file-explicitly)
  - [Reconciling state after a manual publish](#reconciling-state-after-a-manual-publish)
- [cargo-rail does not push](#cargo-rail-does-not-push)
- [Plugin targets](#plugin-targets)
- [What a successful release looks like](#what-a-successful-release-looks-like)

## What owns what

The release is split across three files. Understanding the split is what prevents the failure mode
that produced this repository's `0.2.0` / `0.3.0` / `0.4.0` version cascade and the `98d6cfe`
revert that undid it.

| Concern            | Owner                                              | File                |
| ------------------ | -------------------------------------------------- | ------------------- |
| Version bump       | cargo-rail                                         | `.config/rail.toml` |
| Release commit     | cargo-rail                                         | `.config/rail.toml` |
| Git tag            | cargo-rail                                         | `.config/rail.toml` |
| crates.io upload   | cargo-rail                                         | `.config/rail.toml` |
| Changelog entries  | **A human.** cargo-rail is disabled for this crate | `CHANGELOG.md`      |
| Changelog heading  | `scripts/promote-changelog.sh`                     | `scripts/`          |
| Step orchestration | `Cruxfile` targets                                 | `Cruxfile`          |

The single most important line in `.config/rail.toml` is in the per-crate block:

```toml
[crates.episteme-local.changelog]
skip = true
```

cargo-rail's generated changelog is unusable for this repository. It derives entries from commit
subjects, and the feature work that shipped in 0.2.0 landed in `17998d4`
`chore: checkpoint workspace and Rust documentation` — typed `chore` despite carrying the shipped
feature surface. An auto-generated section would have described the release as chores only. It also
appends its section _below_ older releases instead of replacing the `Unreleased` heading, which
would strand the real notes.

So the changelog is hand-authored under `## Unreleased` and cut to a dated section mechanically.
Write the entry text before the bump; the script only moves a heading.

## Gate order

Run these in order. Each is non-mutating, so each is safe to run at any time.

```text
crux run --strict --plugins ~/.agents/skills/rust-release-orchestrator/scripts/plugins.toml \
  --target release-plan Cruxfile
```

| Step         | Target          | Mutates                                          |
| ------------ | --------------- | ------------------------------------------------ |
| 1. Plan      | `release-plan`  | Nothing. Prints planned version, tag, mutations. |
| 2. Cut notes | `release-bump`  | `CHANGELOG.md`; commits it.                      |
| 3. Release   | `release-apply` | Bump, commit, tag, **upload**.                   |

`release-plan` is the `Cruxfile` default because it is the only release target that cannot change
anything — every other target either mutates the repository or uploads.

Steps 2 and 3 must agree on one version number, pinned in two places:

- the argument to `scripts/promote-changelog.sh` in the `release-bump` target
- the `--bump minor` flag in `release-plan` and `release-apply`

`release-bump` is a separate commit from `release-apply` on purpose. cargo-rail refuses to run
against a dirty tree (`require_clean = true`), and a committed changelog cut is reviewable before
anything is tagged.

## Why `--bump minor`

`pre_1_breaking_bump = "minor"` in `.config/rail.toml`, and 0.2.0 added public API surface: the
intelligence module, the `analyze` and `analyze-batch` subcommands, and two new migrations. Patch
would understate that. For a 1.0+ crate, breaking changes are still a major bump.

## The changelog cut

```text
./scripts/promote-changelog.sh 0.2.0
```

The script refuses to run unless it finds exactly one `## Unreleased` heading and no existing
`## <version>` section, so a release cannot silently publish a changelog with an orphaned
`Unreleased` or a duplicated heading. It only retitles — it never invents, rewords, or reorders
entry text.

## Credentials

The crates.io token is read from 1Password and exported as `CARGO_REGISTRY_TOKEN` for the duration
of the command. cargo-rail's own `cargo publish` subprocess inherits that environment.

Assemble the 1Password reference from a variable rather than writing it out literally:

```bash
SCHEME=op; CARGO_REGISTRY_TOKEN=$(op read "${SCHEME}://cli/cargo-api-token/token") \
  cargo rail release run episteme-local --bump minor --yes
```

The indirection is not stylistic. The pre-commit secret scan treats a literal `op://` reference in
a tracked file as a leaked credential and blocks the commit. Building the URI from `$SCHEME` at run
time keeps that scan satisfied without weakening it.

`op plugin run -- cargo` does **not** work for this, and the failure is confusing enough to be
worth stating outright. The 1Password cargo shell plugin writes `~/.cargo/credentials.toml` with the
token under `[registries.cratebox]` — the private registry declared in `~/.cargo/config.toml` —
not under `[registry]`, which is the key `cargo publish` reads for crates.io. The result is
"no token found" even though `op` exited successfully. Export `CARGO_REGISTRY_TOKEN` directly.

`~/.cargo/config.toml` declares `[registries.cratebox]` but no `replace-with`, so it does not
redirect publishing. crates.io remains the publish target unless that file gains a `replace-with`.

## The apply step is not idempotent

`release-apply` bumps, commits, tags, **and** uploads in one non-idempotent sequence: it reads the
current version and advances it. A retry re-runs that whole sequence, so a failure partway through
an upload advances the version on every attempt.

This is not hypothetical. An earlier revision of the `Cruxfile` carried `retry: {count: 2}` on the
apply step, and a single failed publish produced `0.2.0`, `0.3.0`, and `0.4.0` in sequence —
visible in this repository's history as `5208a60`, `168e56c`, and `cb58668`, undone by the `98d6cfe`
revert. The retry policy was removed in `e4137b6`.

Consequences for anyone running a release:

- **Never blind-retry `release-apply`.** Recover with `release-resume` instead.
- **Verify what actually happened before doing anything.** The error message on a failed upload is
  not trustworthy; see the next section.
- `--bump minor` in the target means a stray re-run does not merely re-upload — it publishes a new
  version number.

## Verify the actual outcome — never trust the error message

During the 0.2.0 release, `cargo rail release resume` ended with:

```text
error: episteme-local v0.2.0 was published but did not become observable within 60s
```

**Both halves of that were wrong.** 0.2.0 had not been published, and the upload had left no trace
on crates.io at all. The registry API, the index, and the download endpoint all agreed the version
was absent. Re-running `release-apply` on the strength of that message would have published
0.3.0 — the same cascade described above.

Cargo-rail polls the sparse index to confirm a version landed, and the index is CDN-cached with
`max-age=600`. A 60-second observation window can therefore read a cache that predates the upload
and misattribute the result. Meanwhile a genuinely failed upload is reported with the same
"was published" phrasing.

So: **always confirm externally before retrying anything.** Three independent checks, strongest
last.

```bash
# 1. Authoritative version list. Requires a User-Agent; crates.io returns 403 without one.
curl -sS -A "episteme-release-check/1.0" https://crates.io/api/v1/crates/episteme-local \
  | jq '{max_version: .crate.max_version, versions: [.versions[].num]}'

# 2. Sparse index. Note it is CDN-cached, so a stale read is not proof of absence.
curl -sS -A "episteme-release-check/1.0" https://index.crates.io/ep/is/episteme-local

# 3. The tarball itself. 403 AccessDenied from S3 means the object does not exist.
curl -sS -o /tmp/dl.crate -w '%{http_code} %{size_download}\n' \
  https://static.crates.io/crates/episteme-local/episteme-local-0.2.0.crate
```

Confirming success also means confirming there is exactly one version, not a duplicate. Compare
`.versions | length` against the version you intended to publish, and check `created_at` — two
attempts at the same version cannot both succeed, so a single `created_at` proves which attempt won.

To confirm the artifact is intact, extract its manifest rather than trusting the upload log:

```bash
tar -xzOf /tmp/dl.crate episteme-local-0.2.0/Cargo.toml | grep -E '^(name|version|edition|license)'
```

Note that `cksum` is `null` in the version object returned by the API, so it is not usable as an
independent integrity check.

## Recovery

When an upload fails partway, `release-resume` is the recovery path — not a re-run of
`release-apply`. cargo-rail persists the completed bump, commit, and tag in
`target/cargo-rail/releases/*.json`, so resuming re-attempts only the upload.

```bash
SCHEME=op; CARGO_REGISTRY_TOKEN=$(op read "${SCHEME}://cli/cargo-api-token/token") \
  cargo rail release resume target/cargo-rail/releases/<state>.json
```

That target carries no retry policy, and `release-apply` does not either. A failed resume must stop,
not silently re-attempt.

### Pass the state file explicitly

The `release-resume` target in the `Cruxfile` selects its state file with:

```bash
$(ls -t target/cargo-rail/releases/*.json | head -1)
```

That is mtime ordering across every state file cargo-rail has ever written in this workspace, and
**the directory accumulates torn state from failed releases**. After the 0.2.0 cycle it held four
files, all still reporting `status: active` and `publication: in_progress`, of which only one
described a real release:

| State file                      | Bump        | Commit    | Meaning                            |
| ------------------------------- | ----------- | --------- | ---------------------------------- |
| `release-a58c6490d796afc9.json` | 0.1.1→0.2.0 | `5208a60` | Abandoned first attempt. Reverted. |
| `release-e1b901ffec6d4a3a.json` | 0.2.0→0.3.0 | `168e56c` | Cascade artifact. Reverted.        |
| `release-23f0097ebf9e8ea4.json` | 0.3.0→0.4.0 | `cb58668` | Cascade artifact. Reverted.        |
| `release-5f769e66f11bd293.json` | 0.1.1→0.2.0 | `477b5ac` | **The real 0.2.0 release.**        |

Selecting by mtime happened to pick the correct one, but only because the failed attempts were
older. It is not a guarantee.

Identify the right file by matching its recorded commit object against the tag you expect to
publish, rather than trusting its timestamp:

```bash
jq -r '[input_filename, .plan.crates[0].bump, .crates[0].commit.object, .status, .crates[0].publication.status] | @tsv' \
  target/cargo-rail/releases/*.json
```

Note the two different paths: the planned bump lives under `.plan.crates[0]`, while the recorded
side effects live under the top-level `.crates[0]`.

The correct entry is the one whose `.crates[0].commit.object` matches the release commit and whose
status is still `in_progress`.

### Reconciling state after a manual publish

If the upload was completed outside cargo-rail — `cargo publish` directly, for example — the
state file still reads `in_progress`, and the next `release-resume` will consider it unfinished.

Re-running `release-resume` is the correct fix, and it is safe: rail observes the existing version
before taking any side effect. Given the published version it settles the state without re-uploading
and reports:

```text
release complete
```

Verify afterwards that no duplicate appeared:

```bash
jq -r '.status, .crates[0].publication.status' target/cargo-rail/releases/<state>.json
# -> complete
# -> complete
```

Do not hand-edit these files. The schema is cargo-rail's, and a hand-written status is not a
reliable substitute for a reconciliation the tool performs against the live registry.

To clear genuinely dead state, delete the files for releases that were reverted. They live under
`/target/` and are untracked, so this is local scratch state only.

## cargo-rail does not push

`push = false` in `.config/rail.toml` means cargo-rail performs **no** remote git operation. After a
successful release, `main` is ahead of `origin/main` and the release tag exists only locally:

```text
git push origin main
git push origin v0.2.0
```

This is what cargo-rail prints when a release completes, and it is not advisory — the publish has
already succeeded by the time you see it.

Push `main` and the tag as separate operations so a tag push cannot drag an unexpected branch state
along with it. Confirm a push will be a fast-forward before running it:

```bash
git rev-list --left-right --count origin/main...main   # left 0 == fast-forward
```

`create_github_release = false`, so no GitHub release is created either.

## Plugin targets

The `Cruxfile` also carries targets backed by the `rust-release-orchestrator` plugin rather than
cargo-rail: `dry-run`, `plugin-release`, `plugin-resume`, and `gates`. They are kept because the
plugin is the more general multi-crate tool, but **for this single-crate repository the cargo-rail
targets are the ones to use** — only cargo-rail additionally owns the version bump and the tag.
`plugin-release` uploads without bumping or tagging, which is exactly the wrong shape for a routine
release here.

```bash
crux run --strict --plugins ~/.agents/skills/rust-release-orchestrator/scripts/plugins.toml \
  --target gates Cruxfile
```

The `--plugins` path is required on every invocation. The release handlers live in a subprocess
plugin rather than crux's built-in registry, and the plugin is not declared in the `Cruxfile`.

The workspace path in those targets is hardcoded to `/Users/joe/dev/episteme`. That is
machine-specific by construction: a `Cruxfile` target executes with a null input, and `--input` is
parsed and then discarded for the multi-target `Cruxfile` form. Options are therefore encoded as
literal YAML per target rather than templated from input.

## What a successful release looks like

Every one of these must hold before the release is considered done:

1. The crates.io API lists the new version, and the count of versions increased by exactly one.
2. The tarball downloads with HTTP 200 and a non-zero size.
3. The tarball's `Cargo.toml` shows the expected name and version.
4. `git push origin main` and the tag push both completed, as a fast-forward.
5. No new `target/cargo-rail/releases/*.json` file is left in an `in_progress` state.

The published artifact is built from the working tree, not from the tagged commit. If the working
tree has commits the tag does not, the published crate will not byte-match the tag. This was true
for 0.2.0: the tag is `477b5ac`, while the upload included `1c2a7d5`, a `Cruxfile`-only change
outside the packaged surface. Harmless, but worth knowing when comparing an artifact to a tag.
