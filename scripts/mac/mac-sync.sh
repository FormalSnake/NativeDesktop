#!/usr/bin/env bash
set -euo pipefail
# One-time (safe to re-run): ensure the bare repo + remote exist.
ssh macbook 'test -d ~/nd.git || git init --bare ~/nd.git >/dev/null'
ssh macbook 'test -d ~/nd || git clone -q ~/nd.git ~/nd'
git remote get-url mac >/dev/null 2>&1 || git remote add mac ssh://macbook/~/nd.git
BRANCH=$(git rev-parse --abbrev-ref HEAD)
git push -f mac "HEAD:refs/heads/$BRANCH"
# `git clone` of a bare repo that had zero refs at clone time (the checkout's
# first-ever bootstrap, before this push ever landed) can leave
# remote.origin.fetch unset — `git fetch` then only updates FETCH_HEAD, not
# refs/remotes/origin/*, and `checkout -B ... origin/$BRANCH` fails with
# "not a commit". Setting the standard refspec is idempotent and safe to
# re-run every sync. Wrapped in `bash -c` because the Mac's login shell is
# fish, not bash — inline `&&`/`$?` chains behave differently there.
ssh macbook "bash -c \"cd ~/nd && git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' && git fetch -q origin && git checkout -q -B '$BRANCH' 'origin/$BRANCH' && git log -1 --oneline\""
echo "MAC_SYNC_OK $BRANCH"
