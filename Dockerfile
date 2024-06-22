FROM messense/rust-musl-cross:x86_64-musl as builder

WORKDIR /usr/src/chimera-md
COPY . .

RUN mkdir /empty && chmod 777 /empty

RUN cargo build --release --target x86_64-unknown-linux-musl

FROM scratch
COPY --from=builder /usr/src/chimera-md/target/x86_64-unknown-linux-musl/release/chimera-md /bin/chimera-md
COPY --from=builder /usr/src/chimera-md/www /data/www
COPY --from=builder /usr/src/chimera-md/templates /data/templates
COPY --from=builder /empty /tmp

EXPOSE 8080

CMD ["/bin/chimera-md"]
