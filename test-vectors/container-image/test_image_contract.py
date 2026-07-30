#!/usr/bin/env python3
"""Static and opt-in runtime contract for the production container image.

The static suite is dependency-free and always runnable. Set
``JIT_IMAGE_RUNTIME=1`` to build the image and exercise both supported bind-mount
identity arrangements with Docker or Podman.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.parse
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = ROOT / "Dockerfile"
IMAGE_TAG = os.environ.get("JIT_IMAGE_TAG", "jit-container-contract:local")
RUNTIME_ENABLED = os.environ.get("JIT_IMAGE_RUNTIME") == "1"


def run(
    *args: str,
    cwd: Path = ROOT,
    check: bool = True,
    timeout: float | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        check=check,
        text=True,
        capture_output=True,
        timeout=timeout,
    )


def container_engine() -> str:
    configured = os.environ.get("CONTAINER_ENGINE")
    if configured:
        if not shutil.which(configured):
            raise RuntimeError(
                f"requested container engine {configured!r} is unavailable"
            )
        probe = run(configured, "info", check=False, timeout=15)
        if probe.returncode != 0:
            detail = (probe.stderr or probe.stdout).strip()
            raise RuntimeError(
                f"requested container engine {configured!r} is unhealthy: {detail}"
            )
        return configured

    for candidate in ("docker", "podman"):
        if shutil.which(candidate):
            probe = run(candidate, "info", check=False, timeout=15)
            if probe.returncode == 0:
                return candidate
    raise RuntimeError(
        "JIT_IMAGE_RUNTIME=1 requires a healthy Docker or Podman engine"
    )


def integer_log_field(log: str, message: str, field: str) -> int:
    plain_log = re.sub(r"\x1b\[[0-9;]*m", "", log)
    line = next((line for line in plain_log.splitlines() if message in line), None)
    if line is None:
        raise AssertionError(f"missing log event {message!r} in:\n{log}")
    value = re.search(rf"\b{re.escape(field)}=(\d+)\b", line)
    if value is None:
        raise AssertionError(f"missing integer field {field!r} in log event: {line}")
    return int(value.group(1))


class StaticImageContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = DOCKERFILE.read_text(encoding="utf-8")
        cls.lower = cls.text.lower()

    def runtime_stage(self) -> str:
        starts = [match.start() for match in re.finditer(r"(?mi)^FROM\s", self.text)]
        self.assertGreaterEqual(len(starts), 3)
        return self.text[starts[-1] :]

    def test_web_build_precedes_embedded_server_build(self) -> None:
        web_build = self.text.index("npm run build")
        web_copy = self.text.index("COPY --from=web-builder")
        server_build = self.text.index("cargo build", web_copy)
        self.assertLess(web_build, web_copy)
        self.assertLess(web_copy, server_build)
        self.assertIn("--bin jit-server", self.text[server_build:])

    def test_runtime_is_one_numeric_nonroot_server_process(self) -> None:
        runtime = self.runtime_stage()
        self.assertRegex(runtime, r"(?m)^USER 10001:10001$")
        self.assertRegex(runtime, r"(?m)^WORKDIR /repo$")
        self.assertIn('exec /usr/local/bin/jit-server --data-dir /repo/.jit', runtime)
        self.assertNotIn("nginx", runtime.lower())
        self.assertNotIn("node", runtime.lower())
        self.assertNotIn("mcp-server", runtime.lower())
        self.assertNotRegex(runtime, r"COPY .*target/release/jit(?:\s|$)")
        self.assertNotIn("tail -f", runtime)

    def test_mount_preflight_is_explicit_and_non_mutating(self) -> None:
        runtime = self.runtime_stage()
        for path in ("/repo", "/repo/.jit"):
            self.assertIn(f"test -d {path}", runtime)
            self.assertIn(f"test -r {path}", runtime)
            self.assertIn(f"test -w {path}", runtime)
            self.assertIn(f"test -x {path}", runtime)
        self.assertNotRegex(runtime.lower(), r"\b(?:chown|chmod)\b")
        self.assertNotIn("jit init", runtime)

    def test_healthcheck_uses_server_health_route(self) -> None:
        self.assertRegex(
            self.text,
            r"HEALTHCHECK[^\n]*\n\s*CMD \[.*http://127\.0\.0\.1:3000/api/health",
        )

    def test_requested_runtime_engine_failure_is_not_silently_skipped(self) -> None:
        command = [
            sys.executable,
            str(Path(__file__).resolve()),
            "RuntimeImageContractTests.test_missing_repository_fails_without_initializing",
        ]
        environment = os.environ.copy()
        environment["CONTAINER_ENGINE"] = "missing-container-engine-3b033738"
        environment.pop("JIT_IMAGE_RUNTIME", None)
        ordinary = subprocess.run(
            command,
            cwd=ROOT,
            env=environment,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(ordinary.returncode, 0, ordinary.stdout + ordinary.stderr)
        self.assertIn("skipped", ordinary.stdout + ordinary.stderr)

        environment["JIT_IMAGE_RUNTIME"] = "1"
        requested = subprocess.run(
            command,
            cwd=ROOT,
            env=environment,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertNotEqual(requested.returncode, 0, requested.stdout + requested.stderr)
        self.assertIn("requested container engine", requested.stdout + requested.stderr)

        environment["CONTAINER_ENGINE"] = sys.executable
        unhealthy = subprocess.run(
            command,
            cwd=ROOT,
            env=environment,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertNotEqual(unhealthy.returncode, 0, unhealthy.stdout + unhealthy.stderr)
        self.assertIn("requested container engine", unhealthy.stdout + unhealthy.stderr)
        self.assertIn("unhealthy", unhealthy.stdout + unhealthy.stderr)


class EventStream:
    """One live SSE connection whose EOF time can be sampled during stop."""

    def __init__(self, host: str, port: int) -> None:
        self.socket = socket.create_connection((host, port), timeout=5)
        self.socket.sendall(
            b"GET /api/events/stream HTTP/1.1\r\n"
            b"Host: localhost\r\nAccept: text/event-stream\r\n\r\n"
        )
        head = self.socket.recv(4096)
        if b"200 OK" not in head or b"text/event-stream" not in head.lower():
            raise AssertionError(f"not a live SSE response: {head!r}")
        self.eof_at: float | None = None

    def wait_for_eof(self, started_at: float) -> None:
        self.socket.settimeout(9)
        try:
            while self.socket.recv(4096):
                pass
            self.eof_at = time.monotonic() - started_at
        finally:
            self.socket.close()


class StalledConnection:
    """An incomplete ordinary request held until the application deadline."""

    def __init__(self, host: str, port: int) -> None:
        self.socket = socket.create_connection((host, port), timeout=5)
        self.socket.sendall(b"GET /api/health HTTP/1.1\r\nHost: localhost\r\n")
        self.closed_at: float | None = None

    def wait_for_close(self, started_at: float) -> None:
        self.socket.settimeout(9)
        try:
            while self.socket.recv(4096):
                pass
            self.closed_at = time.monotonic() - started_at
        finally:
            self.socket.close()


@unittest.skipUnless(RUNTIME_ENABLED, "set JIT_IMAGE_RUNTIME=1 for image runtime tests")
class RuntimeImageContractTests(unittest.TestCase):
    engine: str

    @classmethod
    def setUpClass(cls) -> None:
        cls.engine = container_engine()
        build = run(
            cls.engine,
            "build",
            "--tag",
            IMAGE_TAG,
            "--file",
            "Dockerfile",
            ".",
            check=False,
            timeout=1800,
        )
        if build.returncode != 0:
            raise AssertionError(build.stdout + build.stderr)

    def seed_repository(self, root: Path, name: str) -> tuple[str, str, str]:
        issue_term = f"{name}-issue-term-3b033738"
        document_term = f"{name}-document-term-3b033738"
        document = root / "docs" / "linked.md"
        document.parent.mkdir(parents=True)
        document.write_text(
            f"# Linked fixture\n\n{document_term}\n",
            encoding="utf-8",
        )
        jit = os.environ.get("JIT_BIN", "jit")
        run(jit, "init", "--quiet", cwd=root)
        created = run(
            jit,
            "issue",
            "create",
            issue_term,
            "--description",
            "Container fixture.\n\n## Success Criteria\n\n"
            "- [hard] REQ-01: Serves the mounted fixture.\n",
            "--json",
            cwd=root,
        )
        issue_id = json.loads(created.stdout)["id"]
        run(
            jit,
            "doc",
            "add",
            issue_id,
            "docs/linked.md",
            "--skip-scan",
            "--json",
            cwd=root,
        )
        return issue_id, issue_term, document_term

    def request_json(self, port: int, path: str) -> object:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=5) as response:
            self.assertEqual(response.status, 200)
            return json.load(response)

    def request_text(self, port: int, path: str) -> str:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=5) as response:
            self.assertEqual(response.status, 200)
            return response.read().decode("utf-8")

    def start_container(self, root: Path, name: str, user: str | None) -> tuple[str, int]:
        args = [
            self.engine,
            "run",
            "--detach",
            "--stop-timeout",
            "10",
            "--publish",
            "127.0.0.1::3000",
            "--volume",
            f"{root}:/repo",
            "--name",
            name,
        ]
        if user is not None:
            if self.engine == "podman":
                args.extend(["--userns", "keep-id"])
            args.extend(["--user", user])
        args.append(IMAGE_TAG)
        container_id = run(*args).stdout.strip()
        deadline = time.monotonic() + 20
        port = 0
        while time.monotonic() < deadline:
            mapped = run(
                self.engine,
                "port",
                container_id,
                "3000/tcp",
                check=False,
            ).stdout.strip()
            if mapped:
                port = int(mapped.rsplit(":", 1)[1])
                try:
                    self.request_json(port, "/api/health")
                    return container_id, port
                except OSError:
                    pass
            status = run(
                self.engine,
                "inspect",
                "--format",
                "{{.State.Status}}",
                container_id,
                check=False,
            ).stdout.strip()
            if status == "exited":
                logs = run(self.engine, "logs", container_id, check=False)
                self.fail(f"container exited during startup:\n{logs.stdout}{logs.stderr}")
            time.sleep(0.2)
        self.fail(f"container did not become healthy on mapped port {port}")

    def assert_fixture_contract(self, mapped: bool) -> None:
        with tempfile.TemporaryDirectory(prefix="jit-image-") as directory:
            root = Path(directory)
            fixture = "mapped" if mapped else "default"
            issue_id, issue_term, document_term = self.seed_repository(root, fixture)
            user = f"{os.getuid()}:{os.getgid()}" if mapped else None
            if not mapped:
                for path in [root, *root.rglob("*")]:
                    path.chmod(path.stat().st_mode | 0o007)
            name = f"jit-image-{fixture}-{os.getpid()}"
            container_id = ""
            try:
                container_id, port = self.start_container(root, name, user)
                expected_uid = os.getuid() if mapped else 10001
                identity = run(
                    self.engine,
                    "exec",
                    container_id,
                    "sh",
                    "-c",
                    "printf '%s:%s:%s' \"$(id -u)\" \"$(id -g)\" \"$(cat /proc/1/comm)\"",
                ).stdout
                expected_gid = os.getgid() if mapped else 10001
                self.assertEqual(identity, f"{expected_uid}:{expected_gid}:jit-server")

                root_html = self.request_text(port, "/")
                self.assertIn('<div id="root">', root_html)
                self.assertIn('<div id="root">', self.request_text(port, "/index.html"))

                issue_search = self.request_json(
                    port, "/api/search?" + urllib.parse.urlencode({"q": issue_term})
                )
                self.assertTrue(
                    any(
                        result.get("issue_id") == issue_id
                        and ".jit/issues/" in result["path"]
                        for result in issue_search["results"]
                    ),
                    issue_search,
                )
                document_search = self.request_json(
                    port, "/api/search?" + urllib.parse.urlencode({"q": document_term})
                )
                self.assertTrue(
                    any(result["path"].endswith("docs/linked.md") for result in document_search["results"]),
                    document_search,
                )
                content_path = (
                    f"/api/issues/{issue_id}/documents/"
                    f"{urllib.parse.quote('docs/linked.md', safe='')}/content"
                )
                self.assertIn(document_term, self.request_text(port, content_path))

                probe = f"container-write-{fixture}"
                run(
                    self.engine,
                    "exec",
                    container_id,
                    "sh",
                    "-c",
                    f"printf mounted > /repo/.jit/{probe}",
                )
                self.assertEqual((root / ".jit" / probe).read_text(), "mounted")

                streams = [EventStream("127.0.0.1", port) for _ in range(3)]
                stalled = StalledConnection("127.0.0.1", port)
                started = time.monotonic()
                readers = [
                    threading.Thread(target=stream.wait_for_eof, args=(started,))
                    for stream in streams
                ]
                readers.append(threading.Thread(target=stalled.wait_for_close, args=(started,)))
                for reader in readers:
                    reader.start()
                stop = run(
                    self.engine,
                    "stop",
                    "--time",
                    "10",
                    container_id,
                    timeout=12,
                )
                self.assertEqual(stop.returncode, 0, stop.stdout + stop.stderr)
                elapsed = time.monotonic() - started
                for reader in readers:
                    reader.join(timeout=1)
                    self.assertFalse(reader.is_alive())
                self.assertLess(elapsed, 10)
                self.assertTrue(all(stream.eof_at is not None and stream.eof_at < 2 for stream in streams))
                self.assertIsNotNone(stalled.closed_at)
                self.assertGreaterEqual(stalled.closed_at, 4.0)
                self.assertLess(stalled.closed_at, 8.0)

                exit_code = int(
                    run(
                        self.engine,
                        "inspect",
                        "--format",
                        "{{.State.ExitCode}}",
                        container_id,
                    ).stdout.strip()
                )
                logs = run(self.engine, "logs", container_id, check=False)
                log = logs.stdout + logs.stderr
                self.assertNotEqual(exit_code, 137, log)
                self.assertEqual(exit_code, 0, log)
                started_event = "Shutdown signal received; draining connections"
                expired_event = "Drain deadline expired; force-closing"
                started_count = integer_log_field(log, started_event, "open_connections")
                expired_count = integer_log_field(log, expired_event, "open_connections")
                self.assertEqual(integer_log_field(log, started_event, "drain_deadline_secs"), 5)
                self.assertEqual(integer_log_field(log, expired_event, "drain_deadline_secs"), 5)
                self.assertGreaterEqual(started_count, len(streams) + 1)
                self.assertGreaterEqual(expired_count, 1)
                self.assertGreaterEqual(started_count - expired_count, len(streams))
                self.assertIn("Shutdown complete", log)
            finally:
                if container_id:
                    run(self.engine, "rm", "--force", container_id, check=False)

    def test_default_numeric_identity_runtime(self) -> None:
        self.assert_fixture_contract(mapped=False)

    def test_host_mapped_numeric_identity_runtime(self) -> None:
        self.assert_fixture_contract(mapped=True)

    def test_missing_repository_fails_without_initializing(self) -> None:
        with tempfile.TemporaryDirectory(prefix="jit-image-empty-") as directory:
            root = Path(directory)
            root.chmod(0o777)
            result = run(
                self.engine,
                "run",
                "--rm",
                "--volume",
                f"{root}:/repo",
                IMAGE_TAG,
                check=False,
                timeout=15,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("/repo/.jit must exist", result.stdout + result.stderr)
            self.assertFalse((root / ".jit").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
