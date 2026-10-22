#!/bin/sh
# Puts site/ on the blinker.cool server (nginx serves /var/www/blinker.cool/site)
# by hand. A push to main does it by itself (.github/workflows/site.yml), as
# the user blinker-deploy, so the files go back to that user.
#
#   scripts/deploy-site.sh root@SERVER

set -eu
cd "$(dirname "$0")/.."

HOST=${1:?usage: deploy-site.sh user@server}
# the animations' addresses carry the commit (see V in index.html)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cp -R site/. "$TMP/"
sed "s/__V__/$(git rev-parse --short=8 HEAD)/g" site/index.html > "$TMP/index.html"
rsync -rltz --delete --exclude .DS_Store --exclude version.json "$TMP/" "$HOST:/var/www/blinker.cool/site/"
ssh "$HOST" 'chown -R blinker-deploy:blinker-deploy /var/www/blinker.cool/site && chmod -R u=rwX,go=rX /var/www/blinker.cool/site'
echo "deployed site/ to $HOST"
