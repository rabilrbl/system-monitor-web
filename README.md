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
```bash
docker build -t system-monitor-web .
docker run --rm -p 8765:8765 system-monitor-web
```

## GitHub Container Registry
This repo includes a GitHub Actions workflow that builds and publishes a Docker image to GHCR on pushes to `main` and on tags.
