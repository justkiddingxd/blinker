# blinker-post in a container. Built from the repository root (the page
# uses the fonts in assets/fonts):
#   docker build -f server/Dockerfile -t blinker-post .
FROM rust:1-slim AS build
WORKDIR /src
COPY assets/fonts assets/fonts
COPY server server
RUN cd server && cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 post && mkdir /data && chown post /data
COPY --from=build /src/server/target/release/blinker-post /usr/local/bin/blinker-post
USER post
ENV BIND=0.0.0.0:8080 DATA_DIR=/data
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD ["/bin/sh", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /health HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q ok <&3"]
CMD ["blinker-post"]
