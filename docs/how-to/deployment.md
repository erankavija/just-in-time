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

## Container Deployment (Team Server)

One image carries the whole containerized deployment: a single `jit-server`
process serving the API and the built web UI on port 3000, against a whole
repository bind-mounted at `/repo`. `.jit/` and the project documents linked
from it stay in their repository context, so search and document reads resolve
the same paths they do natively.

Build it from a checkout of this repository:

```bash
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time
docker build -t jit-server:local .
```

The served repository is initialized on the host before the container starts —
`jit init --profile jit-dogfood` for the preferred workflow setup, plain
`jit init` for a methodology-neutral one (see
[Repository Profiles](../reference/profiles.md)). A container started against a
directory without `.jit/` exits with status 78 and creates nothing.

### Mount Identity

The image runs as UID:GID `10001:10001`, a fixed numeric identity that owns
nothing on the host. Running the container unmapped therefore has an ownership
prerequisite: the mounted repository and its `.jit/` directory must already
grant `10001` read, write, and execute permission, which on a repository owned
by someone else means world access.

A deployment normally maps the repository's own owner onto the container user
instead, which is what `JIT_UID` and `JIT_GID` carry:

```bash
export JIT_REPO=/path/to/your/repo
export JIT_UID=$(stat -c '%u' "$JIT_REPO")
export JIT_GID=$(stat -c '%g' "$JIT_REPO")
```

Rootless Podman needs `--userns keep-id` alongside `--user` so the mapped ids
mean the same thing inside the container as they do on the host.

### Compose

`docker-compose.yml` in this repository declares that one service and reads the
three variables above:

```bash
docker compose up -d          # http://localhost:3000 serves the API and web UI
docker compose logs -f
docker compose down
```

`JIT_PORT` publishes the service on a host port other than 3000.

### Plain `docker run` / `podman run`

```bash
docker run -d --name jit-server \
  --user "$JIT_UID:$JIT_GID" \
  --publish 3000:3000 \
  --volume "$JIT_REPO:/repo" \
  jit-server:local
```

`./scripts/test-podman.sh` builds the image and runtime-smokes both identity
arrangements with Podman.

## Backup and Recovery

The container serves the repository in place, so a deployment is backed up the
same way a native one is: archive the repository's `.jit/` directory.

```bash
tar czf jit-backup-$(date +%Y%m%d).tar.gz -C .jit .
```

Restore into a stopped deployment:

```bash
docker compose down
tar xzf jit-backup-YYYYMMDD.tar.gz -C .jit
docker compose up -d
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `JIT_DATA_DIR` | `./.jit` | Native `jit-server` data directory; the container serves `/repo/.jit` from its bind mount |
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |
| `JIT_LOCK_TIMEOUT` | `5` | Lock timeout in seconds (the default is the lock-acquisition timeout in [Runtime Coordination Defaults](../reference/runtime-defaults.md)) |

The Compose service reads three further variables of its own: `JIT_REPO`, the
repository to mount, and `JIT_UID`/`JIT_GID`, the [mount identity](#mount-identity)
it runs as.

## Troubleshooting

### Service won't start

A container that exits with status 78 reports which mount check failed on
stderr: `/repo` missing, `/repo/.jit` missing, or the container identity lacking
read, write, or search permission on either.

```bash
# Check logs
docker compose logs jit-server
journalctl --user -u jit-server -f

# Compare the repository's owner with the identity the container runs as
stat -c '%u:%g' "$JIT_REPO"
docker compose exec jit-server id
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
