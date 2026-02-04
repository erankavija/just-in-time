# Running JIT with Web UI

> **Diátaxis Type:** How-To Guide

JIT is CLI-first—most users just run `jit` commands in their repository. This guide covers adding the web UI for visualization.

## Local Development (Most Common)

Run the API server and web UI separately:

```bash
# Terminal 1: Start API server (points to your repo)
cd /path/to/your/repo
jit-server --data-dir .jit --bind 127.0.0.1:3000

# Terminal 2: Start web UI dev server
cd /path/to/just-in-time/web
npm run dev
```

The web UI runs on `http://localhost:5173` and connects to the API at `localhost:3000`.

### Building the Web UI

```bash
cd web
npm install
npm run build   # Creates dist/ with static files
```

Serve `dist/` with any static file server (nginx, caddy, python -m http.server).

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
# Clone and start
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time
docker compose up -d

# API: http://localhost:3000
# Web: http://localhost:8080
```

### Custom Data Directory

```bash
# Mount your existing repo
docker compose run --rm \
  -v /path/to/your/repo:/data \
  api jit-server
```

## Backup and Recovery

### Backup

```bash
# Docker - backup the data volume
docker run --rm -v jit-data:/data -v $(pwd):/backup alpine \
  tar czf /backup/jit-backup-$(date +%Y%m%d).tar.gz -C /data .

# Native
tar czf jit-backup-$(date +%Y%m%d).tar.gz -C /var/lib/jit .
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
| `JIT_DATA_DIR` | `/data` | Data directory path |
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |
| `JIT_LOCK_TIMEOUT` | `30` | Lock timeout in seconds |

## Troubleshooting

### Service won't start

```bash
# Check logs
docker compose logs api
journalctl -u jit-api -f

# Verify permissions
ls -la /var/lib/jit
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
