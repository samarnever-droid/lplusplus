#!/usr/bin/env sh
# Publish the checked-in wiki source to GitHub's separate <repo>.wiki.git
# repository. Requires WIKI_PUSH_TOKEN with repository contents write access.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
REPOSITORY=${GITHUB_REPOSITORY:-samarnever-droid/lplusplus}
: "${WIKI_PUSH_TOKEN:?Set WIKI_PUSH_TOKEN before publishing the GitHub Wiki}"
python3 "$ROOT/scripts/check_wiki.py"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-wiki.XXXXXX")
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM
git clone "https://x-access-token:${WIKI_PUSH_TOKEN}@github.com/${REPOSITORY}.wiki.git" "$TMP/wiki"
# Keep the wiki repo limited to rendered pages; do not publish source metadata.
find "$TMP/wiki" -mindepth 1 -maxdepth 1 -type f -name '*.md' -delete
cp "$ROOT"/wiki/*.md "$TMP/wiki/"
git -C "$TMP/wiki" add --all
if git -C "$TMP/wiki" diff --cached --quiet; then
  echo "GitHub Wiki is already current"
  exit 0
fi
git -C "$TMP/wiki" -c user.name='lplusplus-bot' -c user.email='lplusplus-bot@users.noreply.github.com' \
  commit -m 'Sync wiki from lplusplus repository'
git -C "$TMP/wiki" push origin HEAD:master
