FROM messense/rust-musl-cross:x86_64-musl AS build
ARG PROXY
ENV http_proxy=$PROXY \
    https_proxy=$PROXY \
    HTTP_PROXY=$PROXY \
    HTTPS_PROXY=$PROXY

WORKDIR /workspace
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl --workspace

FROM alpine:latest
WORKDIR /ygopro
ENV RUST_MIN_STACK=16777216
COPY --from=build /workspace/target/x86_64-unknown-linux-musl/release/ygopro ygopro
COPY --from=build /workspace/target/x86_64-unknown-linux-musl/release/ygopro-toolkits ygopro-toolkits
ENTRYPOINT ["/ygopro/ygopro"]
CMD ["7911"]
