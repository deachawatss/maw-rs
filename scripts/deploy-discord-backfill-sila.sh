#!/usr/bin/env bash
# Deploy discord-backfill (maw-discord-backfill) on oracle-world / sila user.
# Bo GO 2026-07-01 · maclab:gmgrok
set -euo pipefail

BRANCH="${BRANCH:-feat/discord-backfill-rs}"
REPO_URL="${REPO_URL:-https://github.com/MEYD-605/maw-rs.git}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
# cargo install writes to <root>/bin, so the install dir has to end in /bin.
case "$INSTALL_DIR" in
  */bin) ;;
  *) echo "INSTALL_DIR must end in /bin: $INSTALL_DIR" >&2; exit 1 ;;
esac
WORK="${WORK:-$HOME/Code/github.com/MEYD-605/maw-rs}"

echo "== discord-backfill sila deploy =="
echo "branch=$BRANCH install=$INSTALL_DIR"

mkdir -p "$(dirname "$WORK")"
if [ -d "$WORK/.git" ]; then
  git -C "$WORK" fetch origin "$BRANCH"
  git -C "$WORK" checkout "$BRANCH"
  git -C "$WORK" pull --ff-only origin "$BRANCH" || true
else
  git clone --branch "$BRANCH" --depth 1 "$REPO_URL" "$WORK"
fi

cd "$WORK"
# Build and install in one step. cargo places the binary itself, so no target/
# path is written down here. A literal target/release/ path was wrong on this
# repo from 2026-07-17, when .cargo/config.toml moved the target directory to
# /tmp/maw-rs-target; a machine-wide [build] target-dir moves it again. The
# Cargo target directory is a build cache and is not a deploy source.
mkdir -p "$INSTALL_DIR"
cargo install --path crates/maw-discord-backfill --force --root "${INSTALL_DIR%/bin}"

echo "== smoke =="
"$INSTALL_DIR/discord-backfill" whoami
echo "OK: $(command -v discord-backfill) ($(discord-backfill --help 2>&1 | head -1 || true))"