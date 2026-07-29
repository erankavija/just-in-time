# Web production audit evidence

## Result

On 2026-07-29 UTC, the lead ran the following command with approved host network access against the committed lockfile:

```text
npm audit --omit=dev --audit-level=info
```

It exited with code `0` and reported `found 0 vulnerabilities`. The audited lockfile was `web/package-lock.json` with SHA-256:

```text
12ba8dbb5bf6f25fb8cfd945ed02352038ca8760921860a863fd603c2f2a1ad9
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
      "prod": 298,
      "dev": 297,
      "optional": 50,
      "peer": 8,
      "peerOptional": 0,
      "total": 595
    }
  }
}
```

## Scope

This is point-in-time evidence tied to the exact lockfile digest above. It is not a substitute for rerunning the command at release.
