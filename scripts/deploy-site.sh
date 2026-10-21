#!/bin/sh
# Puts site/ on the blinker.cool server (nginx serves /var/www/blinker.cool/site)
# by hand. A push to main does it by itself (.github/workflows/site.yml), as
# the user blinker-deploy, so the files go back to that user.
#
#   scripts/deploy-site.sh root@SERVER

set -eu
cd "$(dirname "$0")/.."

HOST=${1:?usage: deploy-site.sh user@server}
rsync -rltz --delete --exclude .DS_Store site/ "$HOST:/var/www/blinker.cool/site/"
ssh "$HOST" 'chown -R blinker-deploy:blinker-deploy /var/www/blinker.cool/site && chmod -R u=rwX,go=rX /var/www/blinker.cool/site'
echo "deployed site/ to $HOST"
