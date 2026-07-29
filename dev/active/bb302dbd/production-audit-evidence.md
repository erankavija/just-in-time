# MCP production audit evidence

## Result

On 2026-07-29 UTC, the lead ran the following command with approved host network access against the committed lockfile:

```text
npm audit --omit=dev --audit-level=info
```

It exited with code `0` and reported no vulnerabilities. The audited lockfile was `mcp-server/package-lock.json` with SHA-256:

```text
18a19c21e5a34bcc74b1b387c6b54a88f68b1f6ce2ec57ffff7c7baf0909baa1
```

The command returned this JSON payload:

```json
{
  "auditReportVersion": 2,
  "vulnerabilities": {},
  "metadata": {
    "vulnerabilities": {
      "info": 0,
      "low": 0,
      "moderate": 0,
      "high": 0,
      "critical": 0,
      "total": 0
    },
    "dependencies": {
      "prod": 94,
      "dev": 0,
      "optional": 0,
      "peer": 0,
      "peerOptional": 0,
      "total": 93
    }
  }
}
```

## Scope

This is point-in-time evidence tied to the exact lockfile digest above. It is not a substitute for rerunning the command at release.
