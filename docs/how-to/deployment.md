# Deploying the JIT Server and Web UI

> **Diátaxis Type:** How-To Guide

JIT is CLI-first — most work is `jit` commands in a repository. This guide adds
the HTTP surface: the `jit-server` API and the web UI it serves, either from an
installed binary or from one container image.

The `jit-server` binary in the [published
archive](../../INSTALL.md#pre-built-binaries) already carries the built web UI,
so serving a repository natively needs no Node.js toolchain. A release
publishes no container image and no separate web bundle, so the container
deployment below builds its image from a checkout of this repository.

## Serving From an Installed `jit-server`

```bash
cd /path/to/your/repo
jit-server --data-dir .jit --bind 127.0.0.1:3000
```

Open `http://localhost:3000`: the API answers under `/api` and the embedded web
UI is served from `/` on the same origin.

`--web-dir` overrides the embedded assets with a built `dist/` directory, which
is how a source checkout serves a UI it just built:

```bash
jit-server --data-dir .jit --web-dir /path/to/just-in-time/web/dist --bind 127.0.0.1:3000
```

### Building the Web UI

A source checkout builds the bundle with the web workspace's own toolchain
(Node.js — see [optional
dependencies](../../INSTALL.md#optional-dependencies)):

```bash
cd web
npm install
npm run build   # produces dist/ with the static files
```

Compiling `jit-server` after this step embeds the bundle into the binary;
passing `dist/` to `--web-dir` serves it from the filesystem instead. Either
way the UI calls `/api` on its own origin, so a third option is any static
server that also proxies `/api` to `jit-server`.

`npm run dev` starts Vite for frontend asset work. This repository's Vite
configuration declares no `/api` proxy, so the dev server alone does not reach
a separately started `jit-server`; use one of the three same-origin
arrangements above, or add a reverse proxy for `/api`.

## Running as Background Services

### API Server With Systemd

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

### Web UI With Nginx

To serve the built assets from a separate web server, point it at `dist/` and
proxy the API to `jit-server`:

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

## Container Deployment

One image carries the whole containerized deployment: a single `jit-server`
process serving the API and the built web UI on port 3000, against a whole
repository bind-mounted at `/repo`. `.jit/` and the project documents linked
from it stay in their repository context, so search and document reads resolve
the same paths they do natively.

### Build the Image

```bash
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time
docker build -t jit-server:local .
```

The image build compiles the web bundle and the server itself, so the host
needs neither toolchain. `docker-compose.yml` declares the same image name and
build context, so `docker compose up -d` builds it on first use.

### Initialize the Repository First

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

### Remove a Deployment

```bash
docker compose down          # stop and remove the container
docker rmi jit-server:local  # remove the image built above
```

The served repository is a host directory and outlives both.

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
docker compose ps
docker compose logs jit-server
journalctl --user -u jit-server -f

# Compare the repository's owner with the identity the container runs as
stat -c '%u:%g' "$JIT_REPO"
docker compose exec jit-server id
```

### Health check failing

The image's own healthcheck polls the same endpoint, so a failing container
health status and a failing request here have one cause:

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

- [Installation Guide](../../INSTALL.md) - Installing the CLI and the server
- [MCP Integration](mcp-integration.md) - Serving agent clients over MCP
- [Multi-Agent Coordination](multi-agent-coordination.md) - Team workflows
- [Configuration](../reference/configuration.md) - Runtime options
