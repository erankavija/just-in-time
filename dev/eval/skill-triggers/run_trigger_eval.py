#!/usr/bin/env python3
"""Trigger eval runner for jit lead skills (jit:6662f738).

Adapts the skill-creator plugin's trigger-eval technique (temp command file +
headless `claude -p` + stream-json tool-use detection; see the plugin's
`scripts/run_eval.py`) for one confound specific to this repository:
jit-execution-lead and jit-planning-lead are permanently installed as
user-level skills (`~/.claude/skills/<name>`, symlinked into this repo), so
they are present in Claude's `available_skills` list for *every* `claude -p`
invocation, independent of cwd. The plugin's stock runner only counts a
"trigger" if the model invokes a synthetic proxy command carrying a unique
id in its name — but a correctly-behaving model consults the real,
already-installed skill instead (same description, same name minus the
plugin's unique suffix), so the stock runner reports false negatives on
every should-trigger query for these two skills. See README.md in this
directory for the full writeup.

The fix keeps the plugin's methodology intact and only widens the match: a
query is scored as "triggered" if the model calls the Skill tool (or reads
the SKILL.md file directly) for anything whose name contains the skill's
base name — which covers both the real, already-installed skill and the
synthetic proxy this script still creates for parity with the plugin
technique.

Runs each query `--runs-per-query` times (default 3, matching the plugin's
default) to get a reliable per-query trigger rate, and reports pass/fail
per query plus an aggregate rate split by should-trigger vs
should-not-trigger category.

stdlib only; no pip dependencies. Requires the `claude` CLI on PATH.
"""

from __future__ import annotations

import argparse
import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


def parse_skill_md(skill_path: Path) -> tuple[str, str]:
    """Parse (name, description) out of a SKILL.md frontmatter block."""
    content = (skill_path / "SKILL.md").read_text()
    lines = content.split("\n")
    if lines[0].strip() != "---":
        raise ValueError(f"{skill_path}/SKILL.md missing frontmatter opening ---")
    end_idx = next(
        (i for i, l in enumerate(lines[1:], start=1) if l.strip() == "---"), None
    )
    if end_idx is None:
        raise ValueError(f"{skill_path}/SKILL.md missing frontmatter closing ---")

    name = ""
    description = ""
    frontmatter = lines[1:end_idx]
    i = 0
    while i < len(frontmatter):
        line = frontmatter[i]
        if line.startswith("name:"):
            name = line[len("name:") :].strip().strip('"').strip("'")
        elif line.startswith("description:"):
            value = line[len("description:") :].strip()
            if value in (">", "|", ">-", "|-"):
                continuation: list[str] = []
                i += 1
                while i < len(frontmatter) and (
                    frontmatter[i].startswith("  ") or frontmatter[i].startswith("\t")
                ):
                    continuation.append(frontmatter[i].strip())
                    i += 1
                description = " ".join(continuation)
                continue
            description = value.strip('"').strip("'")
        i += 1
    return name, description


def run_single_query(
    query: str,
    skill_name: str,
    description: str,
    timeout: int,
    project_root: Path,
    model: str | None,
) -> bool:
    """Run one query headlessly; return whether the skill triggered."""
    unique_id = uuid.uuid4().hex[:8]
    proxy_name = f"{skill_name}-probe-{unique_id}"
    commands_dir = project_root / ".claude" / "commands"
    command_file = commands_dir / f"{proxy_name}.md"

    try:
        commands_dir.mkdir(parents=True, exist_ok=True)
        indented = "\n  ".join(description.split("\n"))
        command_file.write_text(
            f"---\ndescription: |\n  {indented}\n---\n\n"
            f"# {proxy_name}\n\nThis command handles: {description}\n"
        )

        cmd = [
            "claude",
            "-p",
            query,
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]
        if model:
            cmd.extend(["--model", model])

        env = {k: v for k, v in os.environ.items() if k != "CLAUDECODE"}

        process = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=str(project_root),
            env=env,
        )

        triggered = False
        start_time = time.time()
        buffer = ""
        pending_tool_name = None
        accumulated_json = ""

        try:
            while time.time() - start_time < timeout:
                if process.poll() is not None:
                    remaining = process.stdout.read()
                    if remaining:
                        buffer += remaining.decode("utf-8", errors="replace")
                    break

                ready, _, _ = select.select([process.stdout], [], [], 1.0)
                if not ready:
                    continue

                chunk = os.read(process.stdout.fileno(), 8192)
                if not chunk:
                    break
                buffer += chunk.decode("utf-8", errors="replace")

                while "\n" in buffer:
                    line, buffer = buffer.split("\n", 1)
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError:
                        continue

                    if event.get("type") == "stream_event":
                        se = event.get("event", {})
                        se_type = se.get("type", "")

                        if se_type == "content_block_start":
                            cb = se.get("content_block", {})
                            if cb.get("type") == "tool_use":
                                tool_name = cb.get("name", "")
                                if tool_name in ("Skill", "Read"):
                                    pending_tool_name = tool_name
                                    accumulated_json = ""
                                else:
                                    return False

                        elif se_type == "content_block_delta" and pending_tool_name:
                            delta = se.get("delta", {})
                            if delta.get("type") == "input_json_delta":
                                accumulated_json += delta.get("partial_json", "")
                                if skill_name in accumulated_json:
                                    return True

                        elif se_type in ("content_block_stop", "message_stop"):
                            if pending_tool_name:
                                return skill_name in accumulated_json
                            if se_type == "message_stop":
                                return False

                    elif event.get("type") == "assistant":
                        message = event.get("message", {})
                        for content_item in message.get("content", []):
                            if content_item.get("type") != "tool_use":
                                continue
                            tool_name = content_item.get("name", "")
                            tool_input = content_item.get("input", {})
                            if tool_name == "Skill" and skill_name in tool_input.get(
                                "skill", ""
                            ):
                                triggered = True
                            elif tool_name == "Read" and skill_name in tool_input.get(
                                "file_path", ""
                            ):
                                triggered = True
                            return triggered

                    elif event.get("type") == "result":
                        return triggered
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()

        return triggered
    finally:
        if command_file.exists():
            command_file.unlink()


def run_eval(
    eval_set: list[dict],
    skill_name: str,
    description: str,
    num_workers: int,
    timeout: int,
    project_root: Path,
    runs_per_query: int,
    trigger_threshold: float,
    model: str | None,
) -> dict:
    results = []

    with ThreadPoolExecutor(max_workers=num_workers) as executor:
        future_to_info = {}
        for item in eval_set:
            for run_idx in range(runs_per_query):
                future = executor.submit(
                    run_single_query,
                    item["query"],
                    skill_name,
                    description,
                    timeout,
                    project_root,
                    model,
                )
                future_to_info[future] = (item, run_idx)

        query_triggers: dict[str, list[bool]] = {}
        query_items: dict[str, dict] = {}
        for future in as_completed(future_to_info):
            item, _ = future_to_info[future]
            query = item["query"]
            query_items[query] = item
            query_triggers.setdefault(query, [])
            try:
                query_triggers[query].append(future.result())
            except Exception as e:
                print(f"Warning: query failed: {e}", file=sys.stderr)
                query_triggers[query].append(False)

    for query, triggers in query_triggers.items():
        item = query_items[query]
        trigger_rate = sum(triggers) / len(triggers)
        should_trigger = item["should_trigger"]
        did_pass = (
            trigger_rate >= trigger_threshold
            if should_trigger
            else trigger_rate < trigger_threshold
        )
        results.append(
            {
                "query": query,
                "should_trigger": should_trigger,
                "trigger_rate": trigger_rate,
                "triggers": sum(triggers),
                "runs": len(triggers),
                "pass": did_pass,
            }
        )

    def category_stats(want_should_trigger: bool) -> dict:
        subset = [r for r in results if r["should_trigger"] == want_should_trigger]
        passed = sum(1 for r in subset if r["pass"])
        total = len(subset)
        return {
            "total": total,
            "passed": passed,
            "failed": total - passed,
            "pass_rate": (passed / total) if total else None,
        }

    passed = sum(1 for r in results if r["pass"])
    total = len(results)

    return {
        "skill_name": skill_name,
        "description": description,
        "method": "dev/eval/skill-triggers/run_trigger_eval.py (adapted skill-creator "
        "trigger-eval technique; base-name match instead of unique-proxy match, "
        "see README.md)",
        "runs_per_query": runs_per_query,
        "trigger_threshold": trigger_threshold,
        "results": results,
        "summary": {
            "total": total,
            "passed": passed,
            "failed": total - passed,
            "should_trigger": category_stats(True),
            "should_not_trigger": category_stats(False),
        },
    }


def main():
    parser = argparse.ArgumentParser(
        description="Run trigger evaluation for a jit lead skill's live description"
    )
    parser.add_argument("--eval-set", required=True, help="Path to eval set JSON file")
    parser.add_argument("--skill-path", required=True, help="Path to skill directory")
    parser.add_argument("--out", required=True, help="Path to write results JSON")
    parser.add_argument("--num-workers", type=int, default=6)
    parser.add_argument("--timeout", type=int, default=45, help="Timeout per query run (s)")
    parser.add_argument("--runs-per-query", type=int, default=3)
    parser.add_argument("--trigger-threshold", type=float, default=0.5)
    parser.add_argument(
        "--model", default=None, help="Model to use for claude -p (default: user's configured model)"
    )
    parser.add_argument(
        "--work-dir",
        default=None,
        help="cwd for claude -p; defaults to a fresh temp dir so only "
        "user-level skills are visible (matches how the skill is actually "
        "experienced from any other project)",
    )
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    eval_set = json.loads(Path(args.eval_set).read_text())
    skill_path = Path(args.skill_path)
    if not (skill_path / "SKILL.md").exists():
        print(f"Error: No SKILL.md found at {skill_path}", file=sys.stderr)
        sys.exit(1)

    name, description = parse_skill_md(skill_path)

    cleanup_work_dir = args.work_dir is None
    work_dir = Path(args.work_dir) if args.work_dir else Path(tempfile.mkdtemp(prefix="trigger-eval-"))

    if args.verbose:
        print(f"Evaluating: {name}: {description}", file=sys.stderr)
        print(f"Work dir: {work_dir}", file=sys.stderr)

    try:
        output = run_eval(
            eval_set=eval_set,
            skill_name=name,
            description=description,
            num_workers=args.num_workers,
            timeout=args.timeout,
            project_root=work_dir,
            runs_per_query=args.runs_per_query,
            trigger_threshold=args.trigger_threshold,
            model=args.model,
        )
    finally:
        if cleanup_work_dir:
            shutil.rmtree(work_dir, ignore_errors=True)

    if args.verbose:
        summary = output["summary"]
        print(f"Results: {summary['passed']}/{summary['total']} passed", file=sys.stderr)
        print(f"  should_trigger: {summary['should_trigger']}", file=sys.stderr)
        print(f"  should_not_trigger: {summary['should_not_trigger']}", file=sys.stderr)
        for r in output["results"]:
            status = "PASS" if r["pass"] else "FAIL"
            rate_str = f"{r['triggers']}/{r['runs']}"
            print(
                f"  [{status}] rate={rate_str} expected={r['should_trigger']}: {r['query'][:70]}",
                file=sys.stderr,
            )

    out_path = Path(args.out)
    out_dir = out_path.parent
    out_dir.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=out_dir, prefix=out_path.name + ".", suffix=".tmp")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(json.dumps(output, indent=2) + "\n")
        os.replace(tmp, out_path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise

    print(json.dumps(output["summary"], indent=2))


if __name__ == "__main__":
    main()
