# blinker-post

[Русский](README.ru.md)

The server behind **post.blinker.cool**: blinker uploads a screenshot or a recording, gets a link, and the link opens a page of its own: an image viewer, or a video / sound player with the markers set in blinker's video editor. Files are deleted after 3 days.

The same server can run on your own machine, for your team or just for yourself.

## Run it

You need Docker and a domain pointing at the machine.

```bash
git clone https://github.com/justkiddingxd/blinker
cd blinker/server
```

1. In `docker-compose.yml` set `PUBLIC_URL` to your address (`https://post.your.domain`) and `TOKEN` to a long random string.
2. In `Caddyfile` put the same domain.
3. `docker compose up -d`

Caddy gets the HTTPS certificate by itself. Files go to `./data`.

In blinker: **settings → upload → own server**, address `https://post.your.domain/api/upload`, token = your `TOKEN`. The "check" button sends a test picture.

### Behind nginx instead of Caddy

Drop the `caddy` service, publish the port of `post` (`ports: ["127.0.0.1:8080:8080"]`) and proxy to it:

```nginx
server {
    listen 443 ssl;
    server_name post.your.domain;
    # ssl_certificate ...;
    client_max_body_size 101m;
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_request_buffering off;
    }
}
```

### Without Docker

```bash
cd server && cargo build --release
DATA_DIR=/var/lib/blinker-post PUBLIC_URL=https://post.your.domain TOKEN=... ./target/release/blinker-post
```

## Settings

| Variable | Default | |
|---|---|---|
| `PUBLIC_URL` | `http://localhost:8080` | the address links are made with |
| `TOKEN` | empty | the upload token; empty = anyone can upload |
| `TTL_HOURS` | `72` | how long a file lives |
| `MAX_MB` | `100` | the largest file |
| `DISK_LIMIT_GB` | `20` | uploads stop when the files take this much |
| `UPLOADS_PER_HOUR` | `120` | per address |
| `TRUST_PROXY` | `0` | `1`: the client address comes from `X-Real-IP` (behind a proxy) |
| `BIND` | `0.0.0.0:8080` | |
| `DATA_DIR` | `./data` | |

## What it does

- `POST /api/upload`: the file as the body, the name in `X-Filename`, video markers in `X-Blinker-Markers` (see [`src/upload.rs`](../src/upload.rs) of the app). The type is taken from the file itself: PNG, JPEG, GIF, WebP, MP4, M4A, MP3, WAV, nothing else. The answer: `{"url", "raw_url", "delete_url", "expires"}`.
- `GET /<id>`: the page. Messengers show a preview (Open Graph).
- `GET /f/<id>.<ext>`: the file itself, with range requests for seeking in video.
- `GET /api/delete/<id>?token=…`: deletes the file before its time.

Every file is two files in `DATA_DIR`: `<id>.<ext>` and `<id>.json` with its name, type, markers and expiry time. Expired files are swept every 10 minutes.
