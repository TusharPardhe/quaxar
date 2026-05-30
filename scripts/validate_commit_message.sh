#!/usr/bin/env bash
set -euo pipefail

message_file="${1:-}"

if [[ -z "${message_file}" || ! -f "${message_file}" ]]; then
  echo "usage: $0 <commit-message-file>" >&2
  exit 2
fi

subject="$(sed -n '1p' "${message_file}")"

# Let Git-generated messages through. These are not authored with Commitizen.
if [[ "${subject}" =~ ^(Merge|Revert)\  ]]; then
  exit 0
fi

# Let autosquash commits through so they can be cleaned up by rebase.
if [[ "${subject}" =~ ^(fixup!|squash!)\  ]]; then
  exit 0
fi

pattern='^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)(\([a-z0-9._/-]+\))?(!)?: [^[:space:]].+$'

if [[ "${subject}" =~ ${pattern} ]]; then
  exit 0
fi

cat >&2 <<'EOF'
Invalid commit message.

Commits must follow Commitizen / Conventional Commits:
  <type>(optional-scope): <description>

Allowed types:
  build, chore, ci, docs, feat, fix, perf, refactor, revert, style, test

Examples:
  feat(rpc): add account_objects marker parity
  fix(tx): reject dry-run queue insertion
  test(invariants): cover recursive invalid MPT amounts
EOF

exit 1
