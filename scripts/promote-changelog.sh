#!/usr/bin/env bash
# Promote the CHANGELOG "## Unreleased" section to a dated version section.
#
# cargo-rail owns the version bump and tag, but its own changelog generation is
# unusable here: it derives entries from commit subjects, and the feature merge
# in this repo is typed "chore:", so an auto-generated 0.2.0 section would
# describe 7k lines of shipped features as chores. It also appends its section
# below older releases instead of replacing the Unreleased heading.
#
# So the changelog stays hand-authored. This script performs only the mechanical
# cut: retitle "## Unreleased" to "## <version> - <date>" and leave a fresh
# empty "## Unreleased" above it. It never invents, rewords, or reorders entry
# text -- the notes are already written and were reviewed before the bump.
#
# Usage: promote-changelog.sh <version>
#   e.g. promote-changelog.sh 0.2.0
#
# Exits non-zero if the expected structure is missing, so a release cannot
# silently publish a changelog with an orphaned Unreleased section.

set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
    echo "usage: $(basename "$0") <version>" >&2
    exit 2
fi

CHANGELOG="CHANGELOG.md"
DATE="$(date +%Y-%m-%d)"

[[ -f "$CHANGELOG" ]] || {
    echo "error: $CHANGELOG not found" >&2
    exit 1
}

# The heading must exist and must be unique, or the cut is ambiguous.
COUNT="$(grep -c '^## Unreleased$' "$CHANGELOG" || true)"
if [[ "$COUNT" -ne 1 ]]; then
    echo "error: expected exactly one '## Unreleased' heading, found $COUNT" >&2
    exit 1
fi

# Refuse to run against an already-released version, which would duplicate a
# heading rather than cut one.
if grep -qE "^## ${VERSION//./\\.} " "$CHANGELOG"; then
    echo "error: $CHANGELOG already has a '## $VERSION' section" >&2
    exit 1
fi

# awk holds every line so the retitle can be done in one pass: emit the new
# Unreleased heading, then the dated heading in place of the old one.
awk -v version="$VERSION" -v date="$DATE" '
  /^## Unreleased$/ {
    print "## Unreleased"
    print ""
    print "## " version " - " date
    next
  }
  { print }
' "$CHANGELOG" >"$CHANGELOG.tmp"

mv "$CHANGELOG.tmp" "$CHANGELOG"

echo "promoted Unreleased -> $VERSION - $DATE"
