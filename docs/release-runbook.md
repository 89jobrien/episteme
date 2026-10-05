# Release runbook

How to cut a release of `episteme-local` to crates.io, and how to recover when a publish fails
partway. Written against **cargo-rail 0.25.0** and the `Cruxfile` at the repository root.

## Contents

- [What owns what](#what-owns-what)
- [Gate order](#gate-order)
- [Why `--bump minor`](#why---bump-minor)
- [The changelog cut](#the-changelog-cut)
- [Credentials](#credentials)
- [The apply step is not idempotent](#the-apply-step-is-not-idempotent)
- [Verify the actual outcome — never trust the error message](#verify-the-actual-outcome--never-trust-the-error-message)
  - [The bug, and why it is gone](#the-bug,-and-why-it-is-gone)
- [Recovery](#recovery)
  - [Pass the state file explicitly](#pass-the-state-file-explicitly)
  - [Reconciling state after a manual publish](#reconciling-state-after-a-manual-publish)
- [cargo-rail now pushes git](#cargo-rail-now-pushes-git)
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

| Step         | Target          | Mutates                                                       |
| ------------ | --------------- | ------------------------------------------------------------- |
| 1. Plan      | `release-plan`  | Nothing. Prints planned version, tag, mutations.              |
| 2. Cut notes | `release-bump`  | `CHANGELOG.md`; commits it.                                   |
| 3. Release   | `release-apply` | Bump, commit, **push commit**, **upload**, tag, **push tag**. |

`release-plan` is the `Cruxfile` default because it is the only release target that cannot change
anything — every other target either mutates the repository or uploads.

Steps 2 and 3 must agree on one version number, pinned in two places:

- the argument to `scripts/promote-changelog.sh` in the `release-bump` target
- the `--bump minor` flag in `release-plan` and `release-apply`

`release-bump` is a separate commit from `release-apply` on purpose. The `require_clean = true`
guard that used to force this split was removed in cargo-rail 0.25.0 — previews now permit a dirty
tree and apply rejects paths outside the bound plan. The separate commit is kept anyway: it is the
last point at which the changelog can be reviewed before the version is bumped, committed, and
pushed to origin.

## Why `--bump minor`

`pre_1_breaking_bump = "minor"` in `.config/rail.toml`, and 0.2.0 added public API surface: the
intelligence module, the `analyze` and `classify-batch` subcommands, and two new migrations. Patch
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

During the 0.2.0 release, on cargo-rail 0.17.3, `cargo rail release resume` ended with:

```text
error: episteme-local v0.2.0 was published but did not become observable within 60s
```

**Both halves of that were wrong.** 0.2.0 had not been published, and the upload had never been
attempted. The registry API, the index, and the download endpoint all agreed the version was
absent. Re-running `release-apply` on the strength of that message would have published 0.3.0 —
the same cascade described above.

### The bug, and why it is gone

In 0.17.3, `reconcile_publications` (`src/release/publisher.rs`) short-circuited the upload:

```rust
if publication.status == InProgress {
    self.wait_for_registry(&plan)?;     // polls only — NO upload
    ...
}
publication.status = InProgress;
self.publish_crate(&plan)?;             // the actual upload
```

An `in_progress` state therefore polled for a version that had never been uploaded, then timed out
and blamed the registry. The message was wrong because `wait_for_registry` is reached only _after_
a successful upload on the fresh path, so the code assumed it meant "uploaded, waiting for
propagation."

**This is fixed in 0.25.0.** The `InProgress` early-return is gone, so `publish_crate` runs
unconditionally, and the false message is replaced by an error that fires only when publish
actually failed:

```text
{crate} v{version} remains unobservable on crates.io; resume to reconcile and retry the immutable version
```

Do not downgrade below 0.25.0. The `resume` target is a real upload retry again.

### Confirm externally anyway

The habit is still worth keeping, because a tool reporting success is not evidence a version
exists. Three independent checks, strongest last.

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

Under **0.25.0 or later, `release-resume` is a genuine upload retry.** The 0.17.3 defect that made
resume poll without publishing is fixed — see
[the bug, and why it is gone](#the-bug,-and-why-it-is-gone). cargo-rail persists the bump, commit,
and push state in `target/cargo-rail/releases/*.json`, and resume re-attempts the upload from an
`in_progress` record.

Start by asking rail what it thinks happened:

```bash
cargo rail release status
```

Then choose the recovery path by where the previous attempt stopped:

| Previous attempt stopped                       | Correct recovery                                                                                                                        |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Failed before the commit was pushed            | `release-resume` — nothing remote has changed yet.                                                                                      |
| Failed after `PUSH_RELEASE_COMMIT`, no version | `release-resume`, or `cargo publish` then `resume`.                                                                                     |
| Version is actually published                  | `resume` alone — it will observe it and settle the state.                                                                               |
| Origin is ahead of crates.io                   | Expected. `PUSH_RELEASE_COMMIT` precedes `PUBLISH_CRATE`, so a failed upload leaves the commit on origin. Reconcile, do not force-push. |

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

The first three were deleted once 0.2.0 shipped; only the last remains. The table is kept as a
worked example of what torn state looks like, since the three failures are otherwise invisible in
the repository.

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

## cargo-rail now pushes git

**Changed at 0.25.0.** Under 0.17.3 this repository set `push = false`, and both git pushes were
manual. The option still parses but is deprecated — it resolves to `remote_effects = "none"`,
which is precisely the state `--publish` refuses:

```rust
// src/commands/release.rs, registry_publication_skipped
if release_config.remote_effects == ReleaseRemoteEffects::None {
    return Err("--publish cannot be combined with release.remote_effects = \"none\"");
}
```

So the old setting is not gone, it is **incompatible with publishing**:

| Legacy config                  | Resolves to                  |
| ------------------------------ | ---------------------------- |
| `push = false`, no forge       | `remote_effects = "none"`    |
| `push = true`                  | `remote_effects = "push"`    |
| `create_github_release = true` | `auto` / `github` / `gitlab` |

Two rules follow, both enforced as hard parse errors in `ReleaseConfig`'s deserializer rather than
warnings: `remote_effects` cannot be combined with any of `push`, `create_github_release`, or
`forge`; and `create_github_release = true` requires `push = true`. The legacy path is preserved
only long enough to migrate away from it.

`registry_publication_skipped` carries the same guard at 0.30.1, so this is a deliberate safety
interlock rather than a version quirk: authorizing an irreversible registry upload requires
authorizing remote effects too.

This repository therefore sets `remote_effects = "push"`, and `release-apply` now performs both
pushes. Confirm the resulting order with `release-plan`:

```text
BUMP_VERSION → UPDATE_LOCKFILE → COMMIT_RELEASE → PUSH_RELEASE_COMMIT
→ AWAIT_EXACT_SHA_CHECKS → PUBLISH_CRATE → CREATE_TAG → PUSH_RELEASE_TAGS
```

Two consequences worth internalizing:

- **The release commit reaches `origin` before the crates.io upload.** A failure at the upload stage
  leaves git ahead of the registry. That is recoverable — reconcile with `release-resume` — but
  it inverts the verify-then-push discipline used for 0.2.0, when both pushes were manual.
- **`AWAIT_EXACT_SHA_CHECKS` has nothing to bind to.** This repository has no `.github` workflows.
  The step is planned with `poll = false`, so it should not block, but it is unverified against a
  real apply. If a future release hangs there, that is the first suspect.

No GitHub or GitLab release is created: `remote_effects = "push"` authorizes the git push only, and
the forge release surface is left unset.

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
