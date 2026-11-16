#!/usr/bin/env bash
# Publish Phase 1–2 to GitHub once GH_TOKEN (or gh auth) is available.
# Usage: GH_TOKEN=... ./scripts/publish-github.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! gh auth status -h github.com >/dev/null 2>&1; then
  if [[ -z "${GH_TOKEN:-}${GITHUB_TOKEN:-}" ]]; then
    echo "ERROR: gh is not authenticated. Set GH_TOKEN (repo scope) or run: gh auth login" >&2
    exit 1
  fi
  echo "${GH_TOKEN:-$GITHUB_TOKEN}" | gh auth login --with-token -h github.com
fi

OWNER="${GITHUB_OWNER:-tbwcheatwood-hue}"
NAME="${GITHUB_REPO_NAME:-source}"
DESC='Offline-first autonomous AI coding environment (Cline-parity)'
FEATURE_BRANCH="${FEATURE_BRANCH:-cursor/source-phase-1-2-59e4}"

# Prefer `source`; fall back to `source-ide` if taken by unrelated content.
if gh repo view "$OWNER/$NAME" >/dev/null 2>&1; then
  existing_desc="$(gh repo view "$OWNER/$NAME" --json description -q .description 2>/dev/null || true)"
  existing_empty="$(gh api "repos/$OWNER/$NAME" --jq '.size' 2>/dev/null || echo 1)"
  if [[ "$existing_empty" != "0" ]]; then
    echo "Repo $OWNER/$NAME exists and is non-empty; trying source-ide"
    NAME="source-ide"
  fi
fi

if ! gh repo view "$OWNER/$NAME" >/dev/null 2>&1; then
  gh repo create "$OWNER/$NAME" --private --description "$DESC" --confirm
fi

git remote remove origin 2>/dev/null || true
git remote add origin "https://github.com/$OWNER/$NAME.git"

# Ensure main exists with an initial commit, then push feature branch and open PR.
if ! git show-ref --verify --quiet refs/heads/main; then
  git branch main "$(git rev-list --max-parents=0 HEAD | tail -1)"
fi
# If main is behind feature tip, leave main at root commit for empty-remote bootstrap,
# then push feature as the PR head with full Phase 1–2 history.
ROOT_COMMIT="$(git rev-list --max-parents=0 HEAD | tail -1)"
git branch -f main "$ROOT_COMMIT"

git push -u origin main
git checkout "$FEATURE_BRANCH"
git push -u origin "$FEATURE_BRANCH"

PR_URL="$(gh pr list --repo "$OWNER/$NAME" --head "$FEATURE_BRANCH" --json url -q '.[0].url' || true)"
if [[ -z "$PR_URL" || "$PR_URL" == "null" ]]; then
  PR_URL="$(gh pr create --repo "$OWNER/$NAME" --base main --head "$FEATURE_BRANCH" \
    --title "Phase 1–2: Source core monorepo (daemon, policy, web, desktop)" \
    --body "$(cat <<'EOF'
## Summary
Delivers **Source Phase 1–2 only** (no Phase 3–6):

- `source-core` — shared protocol, network policy types, context models
- `source-daemon` — Axum/SQLite daemon with 3-tier network policy, LLM router, tree-sitter context compactor, shadow git + audit, Plan/Act gates
- `source-web` — React/Vite/Tailwind shell (tier + WebSocket client)
- `source-desktop` — Tauri v2 IPC bridge to the daemon

## Compile / verify
```bash
cargo check -p source-core -p source-daemon -p source-desktop
cargo test  -p source-core -p source-daemon
pnpm install
pnpm --filter source-web typecheck
pnpm --filter source-web build
./scripts/smoke-daemon.sh
```

## Notes
- Data dir: `~/.source/`
- Do not merge until Timothy confirms local verify
- Phases 3–6 intentionally out of scope
EOF
)" )"
fi

echo "REPO=https://github.com/$OWNER/$NAME"
echo "PR=$PR_URL"
echo "MAIN=main"
echo "FEATURE=$FEATURE_BRANCH"