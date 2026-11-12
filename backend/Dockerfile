FROM rust:1.90-slim AS build
WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/sweet-master-api /usr/local/bin/api
ENV PORT=8080 DATABASE_PATH=/data/sweet-master.db
VOLUME /data
EXPOSE 8080
CMD ["api"]
