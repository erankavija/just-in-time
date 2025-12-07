# Deployment Guide

Guide for deploying JIT Issue Tracker in production environments (Linux only).

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Deployment Options](#deployment-options)
- [Docker Compose (Recommended)](#docker-compose-recommended)
- [Systemd Services](#systemd-services)
- [Reverse Proxy Setup](#reverse-proxy-setup)
- [Production Considerations](#production-considerations)
- [Monitoring & Logging](#monitoring--logging)
- [Backup & Recovery](#backup--recovery)

---

## Architecture Overview

JIT consists of 4 components:

```
┌─────────────────────────────────────────────────┐
│                  Reverse Proxy                  │
│            (Nginx/Caddy/Traefik)                │
└────────┬─────────────────────────┬──────────────┘
         │                         │
         │ :80/:443               │ /api/*
         ▼                         ▼
┌─────────────────┐       ┌────────────────┐
│    Web UI       │       │   API Server   │
│  (Static/Nginx) │       │  (jit-server)  │
│    Port 8080    │       │   Port 3000    │
└─────────────────┘       └────────┬───────┘
                                   │
                          ┌────────▼────────┐
                          │  File Storage   │
                          │  (.jit/ dir)    │
                          └─────────────────┘
```

**Additional Components:**
- **CLI**: For admin/automation tasks
- **MCP Server**: For AI agent integration (Node.js)
- **Dispatch**: Coordinator for multi-agent orchestration

---

## Deployment Options

### 1. Docker Compose (Recommended)

**Best for:** Small to medium deployments, easy setup, all-in-one

**Pros:**
- Easy setup and updates
- Consistent environment
- Built-in networking
- Volume management

**Cons:**
- Requires Docker
- Slightly higher resource usage

### 2. Systemd Services

**Best for:** Native deployment, minimal overhead, fine-grained control

**Pros:**
- No container overhead
- Better performance
- Standard Linux service management
- Lower memory footprint

**Cons:**
- Manual dependency management
- Requires more setup

### 3. Kubernetes/Cloud

**Best for:** Large scale, multi-tenant, high availability

**Pros:**
- Horizontal scaling
- Self-healing
- Load balancing

**Cons:**
- Complex setup
- Overkill for most use cases

---

## Docker Compose (Recommended)

### Production Setup

```bash
# 1. Create deployment directory
sudo mkdir -p /opt/jit
cd /opt/jit

# 2. Download docker-compose.yml
wget https://raw.githubusercontent.com/erankavija/just-in-time/main/docker-compose.yml

# 3. Create environment file
cat > .env << EOF
# API Configuration
JIT_DATA_DIR=/data
RUST_LOG=info

# Ports
API_PORT=3000
WEB_PORT=8080
EOF

# 4. Start services
sudo docker-compose up -d

# 5. Initialize repository
sudo docker-compose exec api jit init

# 6. Check health
curl http://localhost:3000/api/health
curl http://localhost:8080
```

### Custom docker-compose.yml for Production

```yaml
version: '3.8'

services:
  api:
    image: ghcr.io/erankavija/just-in-time-api:latest
    container_name: jit-api
    restart: always
    ports:
      - "127.0.0.1:3000:3000"  # Only localhost
    volumes:
      - /var/lib/jit:/data  # Persistent storage
    environment:
      - JIT_DATA_DIR=/data
      - RUST_LOG=info
      - JIT_LOCK_TIMEOUT=10
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://localhost:3000/api/health"]
      interval: 30s
      timeout: 10s
      retries: 3
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"

  web:
    image: ghcr.io/erankavija/just-in-time-web:latest
    container_name: jit-web
    restart: always
    ports:
      - "127.0.0.1:8080:80"  # Only localhost
    depends_on:
      api:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://localhost/"]
      interval: 30s
      timeout: 10s
      retries: 3
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"

volumes:
  jit-data:
    driver: local
    driver_opts:
      type: none
      o: bind
      device: /var/lib/jit
```

### Management Commands

```bash
# View logs
docker-compose logs -f api
docker-compose logs -f web

# Restart services
docker-compose restart

# Update to latest
docker-compose pull
docker-compose up -d

# Backup data
tar -czf jit-backup-$(date +%Y%m%d).tar.gz /var/lib/jit

# Execute CLI commands
docker-compose exec api jit status
docker-compose exec api jit issue list
```

---

## Systemd Services

### Prerequisites

```bash
# Install binaries
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz
sudo mv jit jit-server jit-dispatch /usr/local/bin/
sudo chmod +x /usr/local/bin/jit*

# Create service user
sudo useradd -r -s /bin/false jit

# Create data directory
sudo mkdir -p /var/lib/jit
sudo chown jit:jit /var/lib/jit

# Initialize repository
sudo -u jit JIT_DATA_DIR=/var/lib/jit jit init
```

### API Server Service

Create `/etc/systemd/system/jit-api.service`:

```ini
[Unit]
Description=JIT Issue Tracker API Server
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=jit
Group=jit
Environment="JIT_DATA_DIR=/var/lib/jit"
Environment="RUST_LOG=info"
ExecStart=/usr/local/bin/jit-server
Restart=on-failure
RestartSec=10s

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/jit

# Resource limits
LimitNOFILE=4096
MemoryMax=512M

[Install]
WantedBy=multi-user.target
```

### Web UI with Nginx

Install nginx:

```bash
sudo apt install nginx
```

Create `/etc/nginx/sites-available/jit`:

```nginx
server {
    listen 80;
    server_name jit.example.com;
    
    root /var/www/jit;
    index index.html;

    # Web UI
    location / {
        try_files $uri $uri/ /index.html;
    }

    # API proxy
    location /api/ {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # Static asset caching
    location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg)$ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}
```

Enable and start:

```bash
# Deploy Web UI
sudo mkdir -p /var/www/jit
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-web-ui.tar.gz
sudo tar -xzf jit-web-ui.tar.gz -C /var/www/jit

# Enable site
sudo ln -s /etc/nginx/sites-available/jit /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl restart nginx

# Enable and start API
sudo systemctl daemon-reload
sudo systemctl enable jit-api
sudo systemctl start jit-api

# Check status
sudo systemctl status jit-api
sudo journalctl -u jit-api -f
```

---

## Reverse Proxy Setup

### Nginx with SSL (Let's Encrypt)

```bash
# Install certbot
sudo apt install certbot python3-certbot-nginx

# Get certificate
sudo certbot --nginx -d jit.example.com

# Auto-renewal is configured automatically
sudo systemctl status certbot.timer
```

Updated nginx config with SSL:

```nginx
server {
    listen 443 ssl http2;
    server_name jit.example.com;

    ssl_certificate /etc/letsencrypt/live/jit.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/jit.example.com/privkey.pem;
    
    # SSL configuration
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;
    ssl_prefer_server_ciphers on;

    # ... rest of config
}

server {
    listen 80;
    server_name jit.example.com;
    return 301 https://$server_name$request_uri;
}
```

### Caddy (Automatic HTTPS)

Create `Caddyfile`:

```caddy
jit.example.com {
    reverse_proxy /api/* localhost:3000
    root * /var/www/jit
    file_server
    try_files {path} {path}/ /index.html
    
    encode gzip
    
    header {
        X-Frame-Options SAMEORIGIN
        X-Content-Type-Options nosniff
        X-XSS-Protection "1; mode=block"
    }
}
```

---

## Production Considerations

### Security

```bash
# 1. Firewall rules
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw enable

# 2. Restrict API to localhost only (if behind reverse proxy)
# In docker-compose.yml: "127.0.0.1:3000:3000"
# In systemd: API listens on 127.0.0.1 only

# 3. Regular updates
docker-compose pull && docker-compose up -d
# OR
sudo apt update && sudo apt upgrade
```

### Performance Tuning

```bash
# 1. Enable data volume on fast disk (SSD)
# 2. Adjust file lock timeout for high concurrency
export JIT_LOCK_TIMEOUT=15

# 3. Configure nginx caching
# Add to nginx config:
proxy_cache_path /var/cache/nginx levels=1:2 keys_zone=api_cache:10m max_size=100m;
proxy_cache api_cache;
proxy_cache_valid 200 1m;
```

### High Availability

For HA deployments:

1. **Shared storage**: Mount `.jit/` on NFS/GlusterFS
2. **Load balancer**: HAProxy/nginx upstream for API servers
3. **Read replicas**: Multiple API servers reading same data
4. **Write coordination**: Use file locking (already built-in)

---

## Monitoring & Logging

### Health Checks

```bash
# API health endpoint
curl http://localhost:3000/api/health

# Expected: {"status":"ok"}
```

### Logging

**Docker:**
```bash
docker-compose logs -f --tail=100 api
docker-compose logs -f --tail=100 web
```

**Systemd:**
```bash
sudo journalctl -u jit-api -f
sudo journalctl -u jit-api --since "1 hour ago"
```

### Prometheus Metrics (Future)

Placeholder for future metrics export:

```yaml
# /api/metrics endpoint
- jit_issues_total
- jit_issues_by_state
- jit_api_request_duration_seconds
```

---

## Backup & Recovery

### Automated Backup Script

Create `/usr/local/bin/jit-backup.sh`:

```bash
#!/bin/bash
BACKUP_DIR="/var/backups/jit"
DATA_DIR="/var/lib/jit"
DATE=$(date +%Y%m%d-%H%M%S)

mkdir -p "$BACKUP_DIR"
tar -czf "$BACKUP_DIR/jit-$DATE.tar.gz" -C "$DATA_DIR" .

# Keep last 7 days
find "$BACKUP_DIR" -name "jit-*.tar.gz" -mtime +7 -delete

echo "Backup completed: $BACKUP_DIR/jit-$DATE.tar.gz"
```

Add to cron:

```bash
# Daily backup at 2 AM
0 2 * * * /usr/local/bin/jit-backup.sh
```

### Recovery

```bash
# Stop services
docker-compose down
# OR
sudo systemctl stop jit-api

# Restore backup
tar -xzf jit-backup.tar.gz -C /var/lib/jit/

# Restart services
docker-compose up -d
# OR
sudo systemctl start jit-api
```

---

## Troubleshooting

### API Not Responding

```bash
# Check process
docker-compose ps
sudo systemctl status jit-api

# Check logs
docker-compose logs api
sudo journalctl -u jit-api -n 50

# Check port
sudo netstat -tlnp | grep 3000
```

### File Lock Timeouts

```bash
# Increase timeout
export JIT_LOCK_TIMEOUT=30  # seconds

# Check for stale locks
ls -la /var/lib/jit/.jit/*.lock
```

### Out of Disk Space

```bash
# Check usage
du -sh /var/lib/jit

# Archive old issues
jit issue list --state archived | xargs jit issue delete
```

---

## Next Steps

- Set up monitoring with Prometheus/Grafana
- Configure alerts for service failures
- Implement automated testing in CI/CD
- Document incident response procedures
