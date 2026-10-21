#!/bin/sh
# Puts server/ on the post.blinker.cool machine and restarts it there:
# Docker builds it in /opt/blinker-post (see its docker-compose.yml).
#
#   scripts/deploy-post.sh root@SERVER

set -eu
cd "$(dirname "$0")/.."

HOST=${1:?usage: deploy-post.sh user@server}
rsync -rltz --delete --exclude target --exclude data server "$HOST:/opt/blinker-post/src/"
rsync -rltz --delete assets/fonts "$HOST:/opt/blinker-post/src/assets/"
ssh "$HOST" 'cd /opt/blinker-post && docker compose up -d --build 2>&1 | tail -2 && sleep 2 && curl -fsS http://127.0.0.1:8093/health && echo'
echo "deployed server/ to $HOST"
