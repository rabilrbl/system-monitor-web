# system-monitor-web

A lightweight self-hosted system monitor web UI with a modern mobile-friendly dashboard.

## Files
- `system-monitor.html` - embedded frontend dashboard (served from Rust binary)
- `src/main.rs` - Rust server entrypoint
- `src/lib.rs` - API handlers + telemetry collectors
- `src/model.rs` - JSON models

## Run locally
```bash
cargo run --release
```

Then open <http://localhost:8765>.

## Run with Docker
Most installs should mount the host `/sys` read-only so hardware and power telemetry work correctly.

Build locally and run the image:
```bash
docker build -t system-monitor-web .
docker run --rm -p 8765:8765 -v /sys:/sys:ro system-monitor-web
```

Run the published GHCR image directly:
```bash
docker run --rm -p 8765:8765 -v /sys:/sys:ro ghcr.io/rabilrbl/system-monitor-web:latest
```

Docker Compose with the published GHCR image:
```yaml
services:
  system-monitor-web:
    image: ghcr.io/rabilrbl/system-monitor-web:latest
    container_name: system-monitor-web
    restart: unless-stopped
    ports:
      - "8765:8765"
    volumes:
      - /sys:/sys:ro
```

## Real host system metrics from Docker
If you run the container normally, some metrics will reflect the container namespace rather than the full host.

For the dashboard to see the actual host more accurately, run it with host PID, host network access, and the host `/sys` mount:

```bash
docker run -d \
  --name system-monitor-web \
  --restart unless-stopped \
  --pid=host \
  --network=host \
  -v /sys:/sys:ro \
  ghcr.io/rabilrbl/system-monitor-web:latest
```

Docker Compose equivalent:
```yaml
services:
  system-monitor-web:
    image: ghcr.io/rabilrbl/system-monitor-web:latest
    container_name: system-monitor-web
    restart: unless-stopped
    pid: host
    network_mode: host
    volumes:
      - /sys:/sys:ro
```

### Why these flags matter
- `--pid=host` lets process listings and `/proc`-based stats reflect the host, including Top Workloads process views like CPU and memory leaders
- `--network=host` lets network counters reflect the host network namespace
- `-v /sys:/sys:ro` exposes host sysfs data like battery, GPU, and power-related metrics when available

### Notes
- The app listens on port `8765`, so with `--network=host` you can open:
  - <http://localhost:8765>
  - or `http://<your-host-ip>:8765`
- If you exclude `-v /sys:/sys:ro`, the app still runs, but sysfs-backed telemetry will be incomplete or missing. Battery, thermal, GPU, fan, and other hardware/power-related metrics may disappear or show the container's limited view instead of the real host.
- Some low-level power/GPU sensors can still depend on host kernel permissions and hardware support.
- Hardware support note: this dashboard has only been tested on Intel-based hardware so far. We do not currently have access to AMD-based systems or machines with NVIDIA hardware, so compatibility outside Intel remains unverified.
- On locked-down hosts, you may need additional mounts or privileges for certain sensors, but the command above should cover the common case without going straight to `--privileged`.

## GitHub Container Registry
This repo includes a GitHub Actions workflow that builds and publishes a multi-arch Docker image to GHCR for:
- `linux/amd64`
- `linux/arm64`

It publishes on pushes to `main`, tags, and manual workflow dispatch.

## Rust CI and release binaries
This repo also ships Rust-native CI and release workflows:

- `.github/workflows/rust-ci.yml`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo test --workspace --all-features --locked`
  - Runs on every pull request and push to `main`

- `.github/workflows/release.yml`
  - Triggers on pushes to `main`
  - Reads `Cargo.toml` version and creates/pushes tag `vX.Y.Z` automatically (for example: `version = "0.1.0"` -> tag `v0.1.0`)
  - Fails if that version tag already exists on a different commit (to prevent accidental retagging)
  - Builds release binaries for:
    - `x86_64-unknown-linux-gnu`
    - `aarch64-unknown-linux-gnu`
    - `riscv64gc-unknown-linux-gnu`
    - `x86_64-apple-darwin`
    - `aarch64-apple-darwin`
    - `x86_64-pc-windows-msvc`
    - `aarch64-pc-windows-msvc`
  - Publishes/updates a GitHub Release named `System Monitor Web - vX.Y.Z`
