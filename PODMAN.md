# Using JIT with Podman

Podman is a daemonless container engine that's compatible with Docker. This guide shows how to use JIT with Podman instead of Docker.

## Why Podman?

- **Daemonless**: No background service required
- **Rootless**: Can run containers without root privileges
- **Compatible**: Drop-in replacement for Docker commands
- **Secure**: Better security model than Docker

## Installation

### Debian/Ubuntu
```bash
sudo apt update
sudo apt install podman
```

### Fedora/RHEL
```bash
sudo dnf install podman
```

### Post-Installation: Migrate to SQLite

Podman 5.x still uses the deprecated BoltDB by default. Migrate to SQLite to avoid warnings:

```bash
# Create config directory
mkdir -p ~/.config/containers

# Set SQLite as database backend
cat > ~/.config/containers/containers.conf << 'EOF'
[engine]
database_backend = "sqlite"
EOF

# Reset Podman to apply changes
podman system reset -f

# Verify
podman info | grep databaseBackend
# Should show: databaseBackend: sqlite
```

**Note:** This will remove all existing containers, images, and volumes. Back up important data first.

## Quick Start

### Build Images

```bash
# Build individual images
podman build -t jit-cli -f docker/Dockerfile.cli .
podman build -t jit-api -f docker/Dockerfile.api .
podman build -t jit-web -f docker/Dockerfile.web .

# Or build all-in-one image
podman build -t jit .
```

### Run with Pod (Recommended)

Pods in Podman group containers that share network and resources (similar to Kubernetes pods):

```bash
# Create pod with port mappings
podman pod create --name jit-pod -p 3000:3000 -p 8080:80

# Start API server in pod
podman run -d --pod jit-pod \
  --name jit-api \
  -v jit-data:/data:z \
  -e JIT_DATA_DIR=/data \
  jit-api

# Start Web UI in pod
podman run -d --pod jit-pod \
  --name jit-web \
  jit-web

# Check status
podman pod ps
podman ps --pod
```

### Run Individual Containers

```bash
# API Server
podman run -d --name jit-api \
  -p 3000:3000 \
  -v jit-data:/data:z \
  -e JIT_DATA_DIR=/data \
  jit-api

# Web UI
podman run -d --name jit-web \
  -p 8080:80 \
  jit-web

# CLI (one-off command)
podman run --rm \
  -v jit-data:/data:z \
  jit-cli issue list
```

### Rootless Containers

Podman supports rootless containers out of the box:

```bash
# No sudo needed!
podman run --rm jit-cli --version
```

**Note:** Use `:z` or `:Z` suffix for volume mounts to handle SELinux labels:
- `:z` - shared between containers
- `:Z` - private to container

## Using docker-compose.yml with Podman

### Option 1: podman-compose (Recommended)

Install podman-compose:
```bash
sudo apt install podman-compose
# or
pip3 install podman-compose
```

Use the existing docker-compose.yml:
```bash
podman-compose up -d
podman-compose logs -f
podman-compose down
```

### Option 2: Podman's Docker Compatibility

Create an alias:
```bash
alias docker=podman
alias docker-compose=podman-compose
```

Then use standard Docker commands:
```bash
docker build -t jit .
docker run jit
```

### Option 3: Convert to Podman YAML

Generate Kubernetes-style YAML:
```bash
# Generate from running pod
podman generate kube jit-pod > jit-pod.yaml

# Run from YAML
podman play kube jit-pod.yaml

# Stop
podman play kube --down jit-pod.yaml
```

## Management Commands

### Pod Management

```bash
# List pods
podman pod ps

# List containers in pod
podman ps --pod

# View logs
podman logs -f jit-api

# Stop pod
podman pod stop jit-pod

# Remove pod (and containers)
podman pod rm -f jit-pod

# Restart pod
podman pod restart jit-pod
```

### Container Management

```bash
# List containers
podman ps -a

# Stop container
podman stop jit-api

# Remove container
podman rm jit-api

# Execute command in container
podman exec jit-api jit status

# Interactive shell
podman exec -it jit-api sh
```

### Image Management

```bash
# List images
podman images

# Remove image
podman rmi jit-api

# Inspect image
podman inspect jit-api

# Tag image
podman tag jit-api:latest jit-api:v0.1.0

# Push to registry
podman push jit-api ghcr.io/erankavija/just-in-time-api:latest
```

### Volume Management

```bash
# List volumes
podman volume ls

# Inspect volume
podman volume inspect jit-data

# Remove volume (careful!)
podman volume rm jit-data

# Backup volume
podman run --rm -v jit-data:/data:z -v $(pwd):/backup:z alpine tar czf /backup/jit-backup.tar.gz -C /data .

# Restore volume
podman run --rm -v jit-data:/data:z -v $(pwd):/backup:z alpine sh -c "cd /data && tar xzf /backup/jit-backup.tar.gz"
```

## Systemd Integration

Podman has excellent systemd integration for running containers as services:

### Generate Systemd Unit

```bash
# Generate unit file
podman generate systemd --name jit-api --files --new

# Move to user systemd directory
mkdir -p ~/.config/systemd/user/
mv container-jit-api.service ~/.config/systemd/user/

# Enable and start
systemctl --user daemon-reload
systemctl --user enable container-jit-api.service
systemctl --user start container-jit-api.service

# Check status
systemctl --user status container-jit-api.service
```

### Auto-start on Boot

```bash
# Enable linger (start user services on boot)
sudo loginctl enable-linger $USER

# Your containers will now start on system boot
```

### System-wide Service (with sudo)

```bash
# Generate system unit
sudo podman generate systemd --name jit-api --files --new

# Move to system directory
sudo mv container-jit-api.service /etc/systemd/system/

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable container-jit-api.service
sudo systemctl start container-jit-api.service
```

## Testing

Run the automated test script:

```bash
./scripts/test-podman.sh
```

This will:
1. Build all images
2. Create a pod
3. Start API + Web services
4. Test all endpoints
5. Show logs and status

## Differences from Docker

### Rootless by Default
```bash
# Docker requires sudo (by default)
sudo docker run ...

# Podman works without sudo
podman run ...
```

### No Daemon
```bash
# Docker uses dockerd daemon
sudo systemctl status docker

# Podman is daemonless (no background process)
# Each command is independent
```

### SELinux Labels
```bash
# Docker
docker run -v /path:/data ...

# Podman (with SELinux)
podman run -v /path:/data:z ...
```

### Network Defaults
```bash
# Docker: bridge network by default
# Podman: slirp4netns for rootless, CNI for rootful
```

### Docker Compose
```bash
# Docker
docker-compose up

# Podman (requires podman-compose)
podman-compose up
```

## Troubleshooting

### BoltDB Deprecation Warnings

If you see warnings about BoltDB being deprecated:

```bash
# Migrate to SQLite
mkdir -p ~/.config/containers
echo '[engine]' > ~/.config/containers/containers.conf
echo 'database_backend = "sqlite"' >> ~/.config/containers/containers.conf
podman system reset -f
```

This removes all containers/images, so back up first. After migration:
```bash
podman info | grep databaseBackend  # Should show: sqlite
```

### Permission Denied on Volume Mount

Use `:z` or `:Z` flag:
```bash
podman run -v ./data:/data:z jit-api
```

### Port Already in Use

Check if Docker is running:
```bash
sudo systemctl status docker
sudo systemctl stop docker  # if needed
```

### Rootless Networking Issues

Adjust max user namespaces:
```bash
echo "user.max_user_namespaces=28633" | sudo tee -a /etc/sysctl.conf
sudo sysctl -p
```

### Container Won't Start

Check logs:
```bash
podman logs jit-api
```

Inspect container:
```bash
podman inspect jit-api
```

### SELinux Issues

Temporarily disable (for testing only):
```bash
sudo setenforce 0
```

Or use proper labels:
```bash
podman run -v ./data:/data:Z jit-api
```

## Performance

Podman generally has similar or better performance than Docker:

- **Startup**: Slightly faster (no daemon overhead)
- **Resource usage**: Lower memory footprint (no daemon)
- **Build**: Similar speed to Docker
- **Rootless**: Small performance penalty for networking

## Security

Podman's security advantages:

1. **Rootless containers**: Run without elevated privileges
2. **No daemon**: Reduced attack surface
3. **User namespaces**: Better isolation
4. **SELinux**: Automatic security labeling
5. **No single point of failure**: No daemon to compromise

## Registry Configuration

### GitHub Container Registry

```bash
# Login
podman login ghcr.io

# Pull image
podman pull ghcr.io/erankavija/just-in-time-api:latest

# Push image
podman push jit-api:latest ghcr.io/erankavija/just-in-time-api:latest
```

### Local Registry

```bash
# Run local registry
podman run -d -p 5000:5000 docker.io/library/registry:2

# Tag and push
podman tag jit-api localhost:5000/jit-api
podman push localhost:5000/jit-api
```

## Migration from Docker

### Quick Migration

1. Create alias:
```bash
alias docker=podman
alias docker-compose=podman-compose
```

2. Export Docker images:
```bash
docker save jit-api -o jit-api.tar
```

3. Import to Podman:
```bash
podman load -i jit-api.tar
```

### docker-compose.yml Compatibility

Most docker-compose.yml files work with podman-compose:
```bash
podman-compose -f docker-compose.yml up
```

## Additional Resources

- [Podman Documentation](https://docs.podman.io/)
- [Podman vs Docker](https://docs.podman.io/en/latest/Introduction.html)
- [Rootless Containers](https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md)
- [Podman Compose](https://github.com/containers/podman-compose)

---

## Quick Reference

```bash
# Build
podman build -t jit -f docker/Dockerfile.api .

# Run (rootless)
podman run -d --name jit-api -p 3000:3000 -v jit-data:/data:z jit-api

# Manage
podman ps                    # List running
podman logs -f jit-api       # View logs
podman stop jit-api          # Stop
podman rm jit-api            # Remove

# Pod operations
podman pod create --name jit-pod -p 3000:3000 -p 8080:80
podman run -d --pod jit-pod --name jit-api jit-api
podman pod ps                # List pods
podman pod stop jit-pod      # Stop pod
podman pod rm -f jit-pod     # Remove pod

# Systemd
podman generate systemd --name jit-api --files --new
systemctl --user enable container-jit-api.service
```
