# blinker-post

[English](README.md)

Сервер за **post.blinker.cool**: blinker загружает снимок или запись, получает ссылку, а по ссылке открывается своя страница: просмотр картинки или плеер видео и звука с метками из редактора видео. Файлы удаляются через 3 дня (большие через сутки); когда диск заполнен, первыми уходят самые старые.

Тот же сервер можно поднять у себя: для команды или только для себя.

## Запуск

Нужны Docker и домен, который указывает на машину.

```bash
git clone https://github.com/justkiddingxd/blinker
cd blinker/server
```

1. В `docker-compose.yml` поставь `PUBLIC_URL` — свой адрес (`https://post.твой.домен`), и `TOKEN` — длинную случайную строку.
2. В `Caddyfile` впиши тот же домен.
3. `docker compose up -d`

Caddy сам получит сертификат HTTPS. Файлы лежат в `./data`.

В blinker: **настройки → загрузка → свой сервер**, адрес `https://post.твой.домен/api/upload`, токен — твой `TOKEN`. Кнопка «проверить» отправит тестовую картинку.

### За nginx вместо Caddy

Убери сервис `caddy`, открой порт у `post` (`ports: ["127.0.0.1:8080:8080"]`) и проксируй на него:

```nginx
server {
    listen 443 ssl;
    server_name post.твой.домен;
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

### Без Docker

```bash
cd server && cargo build --release
DATA_DIR=/var/lib/blinker-post PUBLIC_URL=https://post.твой.домен TOKEN=... ./target/release/blinker-post
```

## Настройки

| Переменная | По умолчанию | |
|---|---|---|
| `PUBLIC_URL` | `http://localhost:8080` | адрес, из которого собираются ссылки |
| `TOKEN` | пусто | токен для загрузки; пусто — загружать может кто угодно |
| `TTL_HOURS` | `72` | сколько живёт файл |
| `BIG_MB` | `25` | файлы больше этого… |
| `BIG_TTL_HOURS` | `24` | …живут столько |
| `MAX_MB` | `100` | самый большой файл |
| `DISK_LIMIT_GB` | `20` | дальше этого самые старые файлы удаляются, чтобы освободить место |
| `UPLOADS_PER_HOUR` | `120` | с одного адреса |
| `TRUST_PROXY` | `0` | `1`: адрес клиента берётся из `X-Real-IP` (за прокси) |
| `BIND` | `0.0.0.0:8080` | |
| `DATA_DIR` | `./data` | |

## Что он умеет

- `POST /api/upload`: файл в теле запроса, имя в `X-Filename`, метки видео в `X-Blinker-Markers` (см. [`src/upload.rs`](../src/upload.rs) в приложении). Тип определяется по самому файлу: PNG, JPEG, GIF, WebP, MP4, M4A, MP3, WAV, больше ничего. Ответ: `{"url", "raw_url", "delete_url", "expires"}`.
- `GET /<id>` — страница файла. Мессенджеры показывают превью (Open Graph).
- `GET /f/<id>.<ext>` — сам файл, с запросами диапазонов для перемотки видео.
- `GET /api/delete/<id>?token=…` — удаляет файл раньше срока.

Каждый файл — это два файла в `DATA_DIR`: `<id>.<ext>` и `<id>.json` с именем, типом, метками и временем удаления. Просроченные удаляются каждые 10 минут.
