FROM public.ecr.aws/docker/library/rust:latest AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && printf 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -f target/release/deps/server* target/release/server
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM public.ecr.aws/docker/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/server .
COPY --from=builder /app/migrations ./migrations
ENV RUST_LOG=info
ENV MIGRATIONS_DIR=/app/migrations
EXPOSE 8080
CMD ["./server"]
