#FROM rust:1.79.0-alpine
FROM messense/rust-musl-cross:x86_64-musl

WORKDIR /usr/src/chimera-md
COPY . .

RUN cargo build --release --target x86_64-unknown-linux-musl
RUN cp target/x86_64-unknown-linux-musl/release/chimera-md /usr/bin/chimera-md

EXPOSE 8080

CMD ["/usr/bin/chimera-md"]
