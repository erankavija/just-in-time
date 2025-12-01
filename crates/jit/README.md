# CLI placeholder

This directory is intentionally empty in the scaffold. It will contain the CLI implementation once a language is chosen.

Notes:
- Implementation language: Rust (single binary). Use clap for CLI structure and serde/serde_json for storage.
- The CLI will read/write under the `data/` directory by default, and will support `--data-dir` to override.

Planned CLI commands are described in docs/design.md.