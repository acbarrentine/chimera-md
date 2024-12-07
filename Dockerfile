FROM rust:1.79-alpine AS builder

WORKDIR /usr/src/chimera-md
COPY . .

RUN apk add --no-cache musl-dev
RUN cargo build --release

FROM scratch
COPY --from=builder /usr/src/chimera-md/target/release/chimera-md /bin/chimera-md
COPY --from=builder /usr/src/chimera-md/docker/home /data/home
COPY --from=builder /usr/src/chimera-md/example/templates /data/templates
COPY --from=builder /usr/src/chimera-md/example/www/favicon.ico /data/www/favicon.ico
COPY --from=builder /usr/src/chimera-md/example/www/style/ /data/www/style
COPY --from=builder /usr/src/chimera-md/example/www/icon/ /data/www/icon
COPY --from=builder /usr/src/chimera-md/docker/chimera.toml /data/chimera.toml

EXPOSE 8080

CMD ["/bin/chimera-md"]
