FROM rust:1.79-alpine as builder

WORKDIR /usr/src/chimera-md
COPY . .

RUN apk add --no-cache musl-dev
RUN cargo build --release

FROM scratch
COPY --from=builder /usr/src/chimera-md/target/release/chimera-md /bin/chimera-md
COPY --from=builder /usr/src/chimera-md/www /data/www
COPY --from=builder /usr/src/chimera-md/templates /data/templates
COPY --from=builder /usr/src/chimera-md/style/ /data/style
COPY --from=builder /usr/src/chimera-md/icon/ /data/icon

EXPOSE 8080

CMD ["/bin/chimera-md"]
