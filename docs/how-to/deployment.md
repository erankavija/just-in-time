# Running JIT with Web UI

> **Diátaxis Type:** How-To Guide

JIT is CLI-first—most users just run `jit` commands in their repository. This guide covers adding the web UI for visualization.

## Local Development (Most Common)

Build the web UI, then let `jit-server` serve the files and API from the same
origin:

```bash
cd /path/to/just-in-time/web
npm install
npm run build

cd /path/to/your/repo
jit-server --data-dir .jit --web-dir /path/to/just-in-time/web/dist --bind 127.0.0.1:3000
```

Open `http://localhost:3000`. The current Vite configuration has no `/api`
proxy, so `npm run dev` alone cannot serve this UI against a separate
`jit-server`; use the same-origin setup above or configure a reverse proxy that
serves the built assets and proxies `/api`.

### Building the Web UI

```bash
cd web
npm install
npm run build   # Creates dist/ with static files
```

Serve `dist/` with a static server that also proxies `/api` to `jit-server`, or
pass it to `jit-server --web-dir` as shown above. The UI uses same-origin
`/api` requests.

## Running as Background Services

### API Server with Systemd

```ini
# ~/.config/systemd/user/jit-server.service
[Unit]
Description=JIT API Server

[Service]
Type=simple
WorkingDirectory=/path/to/your/repo
ExecStart=/usr/local/bin/jit-server --data-dir .jit --bind 127.0.0.1:3000
Restart=on-failure

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now jit-server
```

### Web UI with Nginx

After building the web UI (`npm run build`):

```nginx
server {
    listen 8080;
    root /path/to/just-in-time/web/dist;
    index index.html;

    # SPA routing
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Proxy API requests
    location /api/ {
        proxy_pass http://127.0.0.1:3000;
    }
}
```

## Docker Compose (Team Server)

For shared/team deployments with everything containerized:

```bash
# Clone and initialize the named data volume before starting the API service.
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

# `jit-server` requires an initialized JIT data directory. The Compose CLI
# service shares the named `jit-data` volume with the API service.
docker compose run --rm --entrypoint jit cli init --profile jit-dogfood

# Start the API and reverse-proxied Web UI.
docker compose up -d

# API: http://localhost:3000
# Web: http://localhost:8080
```

The profile is the preferred workflow setup and does not require Git or network
access inside the container. Use plain `init` for a methodology-neutral data
volume. See [Repository Profiles](../reference/profiles.md).

### Custom Data Directory

```bash
# Mount the existing JIT data directory itself, not its repository parent.
# `/data` is the Docker configuration; native jit-server defaults to `./.jit`.
docker compose run --rm \
  -v /path/to/your/repo/.jit:/data \
  api
```

## Backup and Recovery

### Backup

```bash
# Docker - backup the data volume
docker run --rm -v jit-data:/data -v $(pwd):/backup alpine \
  tar czf /backup/jit-backup-$(date +%Y%m%d).tar.gz -C /data .

# Native
tar czf jit-backup-$(date +%Y%m%d).tar.gz -C .jit .
```

### Restore

```bash
# Stop services first
docker compose down

# Restore to volume
docker run --rm -v jit-data:/data -v $(pwd):/backup alpine \
  tar xzf /backup/jit-backup-YYYYMMDD.tar.gz -C /data

# Restart
docker compose up -d
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `JIT_DATA_DIR` | `./.jit` | Native `jit-server` data directory; Docker and Compose explicitly set it to `/data` |
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |
| `JIT_LOCK_TIMEOUT` | `5` | Lock timeout in seconds (the default is the lock-acquisition timeout in [Runtime Coordination Defaults](../reference/runtime-defaults.md)) |

## Troubleshooting

### Service won't start

```bash
# Check logs
docker compose logs api
journalctl -u jit-api -f

# Verify permissions
ls -la .jit
```

### Health check failing

```bash
# Test API directly
curl -v http://localhost:3000/api/health

# Check if port is in use
ss -tlnp | grep 3000
```

### Data corruption

```bash
# Run recovery
jit recover

# Or validate
jit validate --fix
```

## See Also

- [Installation Guide](../../INSTALL.md) - Local development setup
- [Multi-Agent Coordination](multi-agent-coordination.md) - Team workflows
- [Configuration](../reference/configuration.md) - Runtime options
