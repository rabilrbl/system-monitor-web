# system-monitor-web

A lightweight self-hosted system monitor web UI with a modern mobile-friendly dashboard.

## Files
- `system-monitor.html` - frontend dashboard
- `system-monitor-server.js` - tiny Node.js server and data API

## Run locally
```bash
node system-monitor-server.js
```

Then open <http://localhost:8765>.

## Run with Docker
Basic container run:
```bash
docker build -t system-monitor-web .
docker run --rm -p 8765:8765 system-monitor-web
```

## Real host system metrics from Docker
If you run the container normally, some metrics will reflect the container namespace rather than the full host.

For the dashboard to see the actual host more accurately, run it with host PID and host network access:

```bash
docker run -d \
  --name system-monitor-web \
  --restart unless-stopped \
  --pid=host \
  --network=host \
  -v /sys:/sys:ro \
  ghcr.io/rabilrbl/system-monitor-web:latest
```

### Why these flags matter
- `--pid=host` lets process listings and `/proc`-based stats reflect the host
- `--network=host` lets network counters reflect the host network namespace
- `-v /sys:/sys:ro` exposes host sysfs data like battery, GPU, and power-related metrics when available

### Notes
- The app listens on port `8765`, so with `--network=host` you can open:
  - <http://localhost:8765>
  - or `http://<your-host-ip>:8765`
- Some low-level power/GPU sensors can still depend on host kernel permissions and hardware support.
- Hardware support note: this dashboard has only been tested on Intel-based hardware so far. We do not currently have access to AMD-based systems or machines with NVIDIA hardware, so compatibility outside Intel remains unverified.
- On locked-down hosts, you may need additional mounts or privileges for certain sensors, but the command above should cover the common case without going straight to `--privileged`.

## GitHub Container Registry
This repo includes a GitHub Actions workflow that builds and publishes a multi-arch Docker image to GHCR for:
- `linux/amd64`
- `linux/arm64`

It publishes on pushes to `main`, tags, and manual workflow dispatch.
