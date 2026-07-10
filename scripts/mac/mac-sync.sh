#!/usr/bin/env bash
set -euo pipefail
# One-time (safe to re-run): ensure the bare repo + remote exist.
ssh macbook 'test -d ~/nd.git || git init --bare ~/nd.git >/dev/null'
ssh macbook 'test -d ~/nd || git clone -q ~/nd.git ~/nd'
git remote get-url mac >/dev/null 2>&1 || git remote add mac ssh://macbook/~/nd.git
BRANCH=$(git rev-parse --abbrev-ref HEAD)
git push -f mac "HEAD:refs/heads/$BRANCH"
ssh macbook "cd ~/nd && git fetch -q origin && git checkout -q -B '$BRANCH' 'origin/$BRANCH' && git log -1 --oneline"
echo "MAC_SYNC_OK $BRANCH"
