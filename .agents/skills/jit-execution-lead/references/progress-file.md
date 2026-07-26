# Progress file

Persist the wave plan to `progress.json` in the epic's artifact directory, resolved via `jit doc dir <epic-id> dev/active`:

```json
{
  "epic_id": "<full-id>",
  "epic_short_id": "<short-id>",
  "current_wave": 1,
  "waves": [
    {
      "wave_number": 1,
      "issues": [
        {"id": "<full-id>", "short_id": "<short-id>", "title": "...", "classification": "implementation", "status": "pending"}
      ]
    }
  ],
  "created_during_execution": [],
  "escalations": [],
  "rework_counts": {},
  "started_at": "<ISO-8601>"
}
```

Update after every wave: advance `current_wave`, set per-issue `status`, record
`rework_counts` and `escalations`. Commit alongside JIT state per jit-manage
state-commit-patterns. On epic completion, archive it per the project's
documentation config.
