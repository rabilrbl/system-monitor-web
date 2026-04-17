FROM rust:1.94-alpine AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY system-monitor.html ./system-monitor.html
RUN cargo build --release

FROM alpine:3.22
WORKDIR /app
COPY --from=build /app/target/release/system-monitor-web /usr/local/bin/system-monitor-web
ENV PORT=8765
EXPOSE 8765
CMD ["/usr/local/bin/system-monitor-web"]
