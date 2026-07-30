# Replacement-image smoke before the Docker topology cutover

Evidence that the one supported server image builds and serves a bind-mounted
repository, recorded while the superseded `docker/` definitions were still
present. The deletions land in the commits that follow this record.

## Conditions

- Date: 2026-07-30 UTC
- Tree: `9a7fd536` with `docker/Dockerfile.api`, `docker/Dockerfile.web`,
  `docker/Dockerfile.cli`, `docker/entrypoint.sh`, and `docker/nginx.conf` still
  tracked.
- Engine: `podman version 6.0.1`. This host has no Docker daemon, so Podman is
  the runtime the contract suite was pointed at.

## Contract suite

The suite in `test-vectors/container-image/test_image_contract.py` builds the
root `Dockerfile` and exercises both supported identity arrangements, the SPA
and API routes, repository-relative document search, the bounded drain, and the
refusal to start against an uninitialized mount.

```text
$ CONTAINER_ENGINE=podman JIT_IMAGE_RUNTIME=1 \
    python3 test-vectors/container-image/test_image_contract.py
test_default_numeric_identity_runtime (__main__.RuntimeImageContractTests.test_default_numeric_identity_runtime) ... ok
test_host_mapped_numeric_identity_runtime (__main__.RuntimeImageContractTests.test_host_mapped_numeric_identity_runtime) ... ok
test_missing_repository_fails_without_initializing (__main__.RuntimeImageContractTests.test_missing_repository_fails_without_initializing) ... ok
test_healthcheck_uses_server_health_route (__main__.StaticImageContractTests.test_healthcheck_uses_server_health_route) ... ok
test_mount_preflight_is_explicit_and_non_mutating (__main__.StaticImageContractTests.test_mount_preflight_is_explicit_and_non_mutating) ... ok
test_requested_runtime_engine_failure_is_not_silently_skipped (__main__.StaticImageContractTests.test_requested_runtime_engine_failure_is_not_silently_skipped) ... ok
test_runtime_is_one_numeric_nonroot_server_process (__main__.StaticImageContractTests.test_runtime_is_one_numeric_nonroot_server_process) ... ok
test_web_build_precedes_embedded_server_build (__main__.StaticImageContractTests.test_web_build_precedes_embedded_server_build) ... ok

----------------------------------------------------------------------
Ran 8 tests in 170.726s

OK
```

Exit code `0`.

## Repository-mount transcript

The same arrangement the Compose service and the migrated documentation
describe: a repository owner derived with `stat`, mapped onto the container
identity, over a whole-repository bind mount.

```text
$ podman build --tag jit-server:cutover-smoke --file Dockerfile .
[3/3] COMMIT jit-server:cutover-smoke
--> 48de40714236
Successfully tagged localhost/jit-server:cutover-smoke

$ REPO=$(mktemp -d) && (cd "$REPO" && jit init --quiet)
$ JIT_UID=$(stat -c '%u' "$REPO") JIT_GID=$(stat -c '%g' "$REPO")
derived JIT_UID=1000 JIT_GID=1000

$ podman run --detach --userns keep-id --user "$JIT_UID:$JIT_GID" \
    --publish 127.0.0.1:3111:3000 --volume "$REPO:/repo" jit-server:cutover-smoke
0561c7154fe3e1d92d4b952f015c4c365b972a470bf73b4c0bb7029fe54ada0e

$ curl -sS http://127.0.0.1:3111/api/health
{"project_name":"repo","service":"jit-api","status":"ok","version":"1.0.0"}

$ curl -sS -o /dev/null -w 'web-ui HTTP %{http_code} %{content_type}\n' http://127.0.0.1:3111/
web-ui HTTP 200 text/html

$ podman exec "$CID" sh -c 'printf "%s %s\n" "$(id -u):$(id -g)" "$(cat /proc/1/comm)"'
1000:1000 jit-server

$ podman stop --time 10 "$CID" && podman inspect --format '{{.State.ExitCode}}' "$CID"
0
```

One process is PID 1, it runs as the mapped repository owner, one port serves
both the API and the web UI, and the stop exits `0` inside the stop timeout.

## Rerunning it

`scripts/test-podman.sh` drives the same suite under Podman.
