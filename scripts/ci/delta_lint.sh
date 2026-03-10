#!/usr/bin/env bash
set -euo pipefail
# Delta lint: only fail on clippy warnings/errors that touch changed lines.
# Compares the current branch against the merge base with origin/main.

CLIPPY_OUT=""
DIFF_OUT=""

cleanup() {
    [ -n "$CLIPPY_OUT" ] && rm -f "$CLIPPY_OUT"
    [ -n "$DIFF_OUT" ] && rm -f "$DIFF_OUT"
}
trap cleanup EXIT

# Compute merge base
BASE=$(git merge-base origin/main HEAD)

# Find changed .rs files
CHANGED_RS=$(git diff --name-only "$BASE" -- '*.rs' || true)
if [ -z "$CHANGED_RS" ]; then
    echo "==> delta lint: no .rs files changed, skipping"
    exit 0
fi

echo "==> delta lint: checking changed lines since $(echo "$BASE" | head -c 10)..."

# Extract unified-0 diff for changed line ranges
DIFF_OUT=$(mktemp "${TMPDIR:-/tmp}/ironclaw-diff.XXXXXX")
git diff --unified=0 "$BASE" -- '*.rs' > "$DIFF_OUT"

# Run clippy with JSON output
CLIPPY_OUT=$(mktemp "${TMPDIR:-/tmp}/ironclaw-clippy.XXXXXX")
cargo clippy --all-targets --message-format=json -- -D warnings > "$CLIPPY_OUT" 2>/dev/null || true

# Filter clippy diagnostics against changed line ranges
python3 - "$DIFF_OUT" "$CLIPPY_OUT" <<'PYEOF'
import json
import re
import sys
import os

def parse_diff(diff_path):
    """Parse unified-0 diff to extract {file: [[start, end], ...]} changed ranges."""
    changed = {}
    current_file = None
    with open(diff_path) as f:
        for line in f:
            # Match +++ b/path/to/file.rs
            m = re.match(r'^\+\+\+ b/(.+)$', line)
            if m:
                current_file = m.group(1)
                if current_file not in changed:
                    changed[current_file] = []
                continue
            # Match @@ hunk headers: @@ -old,count +new,count @@
            m = re.match(r'^@@ .+ \+(\d+)(?:,(\d+))? @@', line)
            if m and current_file:
                start = int(m.group(1))
                count = int(m.group(2)) if m.group(2) is not None else 1
                if count == 0:
                    continue
                end = start + count - 1
                changed[current_file].append([start, end])
    return changed

def normalize_path(path):
    """Normalize absolute path to relative (from repo root)."""
    if os.path.isabs(path):
        cwd = os.getcwd()
        if path.startswith(cwd):
            return os.path.relpath(path, cwd)
    return path

def in_changed_range(file_path, line, changed_ranges):
    """Check if file:line falls within any changed range."""
    rel = normalize_path(file_path)
    ranges = changed_ranges.get(rel)
    if not ranges:
        return False
    return any(start <= line <= end for start, end in ranges)

def main():
    diff_path = sys.argv[1]
    clippy_path = sys.argv[2]

    changed_ranges = parse_diff(diff_path)

    blocking = []
    baseline = []

    with open(clippy_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue

            if msg.get("reason") != "compiler-message":
                continue

            cm = msg.get("message", {})
            level = cm.get("level", "")
            if level not in ("warning", "error"):
                continue

            # Get primary span
            spans = cm.get("spans", [])
            primary = None
            for s in spans:
                if s.get("is_primary"):
                    primary = s
                    break
            if not primary:
                if spans:
                    primary = spans[0]
                else:
                    continue

            file_name = primary.get("file_name", "")
            line_start = primary.get("line_start", 0)
            rendered = cm.get("rendered", "").strip()

            if in_changed_range(file_name, line_start, changed_ranges):
                blocking.append(rendered)
            else:
                baseline.append(rendered)

    if baseline:
        print(f"\n--- Baseline warnings (not in changed lines, informational) [{len(baseline)}] ---")
        for w in baseline[:10]:
            print(w)
        if len(baseline) > 10:
            print(f"  ... and {len(baseline) - 10} more")

    if blocking:
        print(f"\n*** BLOCKING: {len(blocking)} issue(s) in changed lines ***")
        for w in blocking:
            print(w)
        sys.exit(1)
    else:
        print("\n==> delta lint: passed (no issues in changed lines)")
        sys.exit(0)

if __name__ == "__main__":
    main()
PYEOF
