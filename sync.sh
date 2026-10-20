#!/usr/bin/env bash
# sync.sh — rsync the braces-rs working tree to the Ubuntu box (gsai) for
# build/bench. Mirrors the proxy derivation in box-ssh.sh (never hardcode
# the proxy host). Box worktree: /mnt/sdb1/braces-rs/src-repo (NTFS-safe:
# source only; CARGO_TARGET_DIR lives on ext4 — see bench/run-on-box.sh).
set -euo pipefail

BOX_USER_HOST="${1:-muse@100.104.140.2}"
SRC="$(cd "$(dirname "$0")" && pwd)/"
DEST="/mnt/sdb1/braces-rs/src-repo/"

ph="${HTTPS_PROXY#*://}"
ph="${ph#*@}"
ph="${ph%%:*}"

rsync -a --no-perms --no-times --delete \
  --exclude '.git/' \
  --exclude 'bench/results.json' \
  --exclude 'target/' \
  --exclude 'node_modules/' \
  --exclude '*.node' \
  --exclude 'napi/binding.js' \
  --exclude 'napi/binding.d.ts' \
  -e "ssh -i /home/hatch/.ssh/id_ed25519 -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ProxyCommand=\"nc -X connect -x ${ph}:3130 %h %p\"" \
  "$SRC" "${BOX_USER_HOST}:${DEST}"
echo "synced -> ${BOX_USER_HOST}:${DEST}"
