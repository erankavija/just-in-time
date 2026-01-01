# Integration Patterns for jit-dispatch

This document shows common patterns for integrating custom orchestrators and agents with jit.

## Pattern 1: Simple Polling Agent

**Use case:** Single agent that polls for work

```bash
#!/bin/bash
# simple-agent.sh

AGENT_ID="agent:my-worker"

while true; do
    # Get my assigned work
    WORK=$(jit query assignee "$AGENT_ID" --json)
    
    # Process each task
    echo "$WORK" | jq -r '.issues[].id' | while read -r ISSUE_ID; do
        echo "Processing $ISSUE_ID..."
        
        # Do the actual work here
        ./process-task.sh "$ISSUE_ID"
        
        # Mark complete
        jit issue update "$ISSUE_ID" --state done
    done
    
    sleep 60
done
```

## Pattern 2: GitHub Actions Integration

**Use case:** Use GitHub Actions as the execution environment

```yaml
# .github/workflows/jit-worker.yml
name: JIT Worker

on:
  schedule:
    - cron: '*/5 * * * *'  # Every 5 minutes
  workflow_dispatch:

jobs:
  claim-and-execute:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup jit
        run: |
          cargo install --path crates/jit
      
      - name: Claim next task
        id: claim
        run: |
          ISSUE_ID=$(jit issue claim-next "agent:github-actions" || echo "")
          echo "issue_id=$ISSUE_ID" >> $GITHUB_OUTPUT
      
      - name: Execute task
        if: steps.claim.outputs.issue_id != ''
        run: |
          echo "Processing issue: ${{ steps.claim.outputs.issue_id }}"
          # Your task execution logic here
          
      - name: Mark complete
        if: steps.claim.outputs.issue_id != ''
        run: |
          jit issue update "${{ steps.claim.outputs.issue_id }}" --state done
```

## Pattern 3: Priority-Based Multi-Agent

**Use case:** Multiple agents with different capabilities

```python
#!/usr/bin/env python3
# priority-dispatcher.py

import subprocess
import json

AGENT_CAPABILITIES = {
    "agent:fast-cpu": ["build", "test"],
    "agent:gpu-runner": ["train", "inference"],
    "agent:security-scanner": ["security", "audit"],
}

def get_ready_issues():
    result = subprocess.run(
        ["jit", "query", "ready", "--json"],
        capture_output=True, text=True
    )
    return json.loads(result.stdout)["issues"]

def match_agent_to_issue(issue):
    """Match issue to appropriate agent based on tags"""
    issue_tags = issue.get("context", {}).get("tags", [])
    
    for agent_id, capabilities in AGENT_CAPABILITIES.items():
        if any(tag in capabilities for tag in issue_tags):
            return agent_id
    
    # Default agent
    return "agent:default-worker"

def dispatch():
    ready = get_ready_issues()
    
    for issue in ready:
        agent_id = match_agent_to_issue(issue)
        
        subprocess.run([
            "jit", "issue", "claim",
            issue["id"], agent_id
        ])
        
        print(f"Assigned {issue['id']} to {agent_id}")

if __name__ == "__main__":
    dispatch()
```

## Pattern 4: Webhook-Driven Orchestration

**Use case:** React to events instead of polling

```javascript
// webhook-orchestrator.js
const express = require('express');
const { exec } = require('child_process');
const util = require('util');
const execPromise = util.promisify(exec);

const app = express();
app.use(express.json());

// Listen for jit events
app.post('/webhook/jit-event', async (req, res) => {
  const event = req.body;
  
  console.log('Received event:', event.type);
  
  // When an issue becomes ready, dispatch it
  if (event.type === 'issue_state_changed' && event.to === 'ready') {
    try {
      const { stdout } = await execPromise(
        `jit issue claim-next agent:webhook-worker`
      );
      console.log('Claimed issue:', stdout);
    } catch (err) {
      console.error('Failed to claim:', err);
    }
  }
  
  // When an issue is completed, check for newly unblocked issues
  if (event.type === 'issue_completed') {
    console.log('Issue completed, checking for unblocked issues...');
    // Auto-transition handles this now!
  }
  
  res.sendStatus(200);
});

app.listen(3000, () => {
  console.log('Webhook orchestrator listening on port 3000');
});
```

## Pattern 5: Kubernetes CronJob Orchestrator

**Use case:** Run orchestrator as a k8s scheduled job

```yaml
# k8s/jit-orchestrator-cronjob.yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: jit-orchestrator
spec:
  schedule: "*/5 * * * *"  # Every 5 minutes
  jobTemplate:
    spec:
      template:
        spec:
          containers:
          - name: orchestrator
            image: jit-dispatch:latest
            command: ["jit-dispatch", "once"]
            volumeMounts:
            - name: jit-repo
              mountPath: /workspace
            - name: config
              mountPath: /config
            env:
            - name: JIT_REPO
              value: /workspace
          volumes:
          - name: jit-repo
            persistentVolumeClaim:
              claimName: jit-repo-pvc
          - name: config
            configMap:
              name: dispatch-config
          restartPolicy: OnFailure
```

## Pattern 6: Stale Issue Detection

**Use case:** Reclaim issues that have been stuck for too long

```python
#!/usr/bin/env python3
# stale-detector.py

import subprocess
import json
from datetime import datetime, timedelta

STALE_THRESHOLD_HOURS = 4

def get_events():
    result = subprocess.run(
        ["jit", "events", "tail", "--json"],
        capture_output=True, text=True
    )
    return json.loads(result.stdout)

def find_stale_issues():
    """Find issues claimed more than N hours ago"""
    events = get_events()
    claimed_issues = {}
    
    # Build map of issue -> last claimed time
    for event in events:
        if event["type"] == "issue_claimed":
            issue_id = event["issue_id"]
            timestamp = datetime.fromisoformat(event["timestamp"])
            claimed_issues[issue_id] = timestamp
    
    # Check for stale issues
    now = datetime.now()
    stale = []
    
    for issue_id, claimed_at in claimed_issues.items():
        age = now - claimed_at
        if age > timedelta(hours=STALE_THRESHOLD_HOURS):
            stale.append((issue_id, age))
    
    return stale

def reclaim_stale_issue(issue_id):
    """Release and reclaim a stale issue"""
    subprocess.run(["jit", "issue", "release", issue_id, "timeout"])
    print(f"Released stale issue: {issue_id}")

if __name__ == "__main__":
    stale_issues = find_stale_issues()
    
    for issue_id, age in stale_issues:
        print(f"Stale issue {issue_id} (age: {age})")
        reclaim_stale_issue(issue_id)
```

## Pattern 7: Federated Multi-Repo Orchestration

**Use case:** Coordinate work across multiple jit repositories

```python
#!/usr/bin/env python3
# multi-repo-orchestrator.py

import subprocess
import json
from pathlib import Path

REPOS = [
    "/workspace/backend-repo",
    "/workspace/frontend-repo",
    "/workspace/infra-repo",
]

def query_repo(repo_path, agent_id):
    """Query a specific repo for work"""
    result = subprocess.run(
        ["jit", "query", "ready", "--json"],
        cwd=repo_path,
        capture_output=True,
        text=True
    )
    issues = json.loads(result.stdout)["issues"]
    
    # Add repo context to each issue
    for issue in issues:
        issue["repo"] = repo_path
    
    return issues

def dispatch_across_repos(agent_id):
    """Find and claim highest priority work across all repos"""
    all_issues = []
    
    for repo in REPOS:
        issues = query_repo(repo, agent_id)
        all_issues.extend(issues)
    
    if not all_issues:
        return None
    
    # Sort by priority
    priority_order = {"critical": 0, "high": 1, "normal": 2, "low": 3}
    all_issues.sort(
        key=lambda i: priority_order.get(i.get("priority", "normal"), 4)
    )
    
    # Claim highest priority
    top_issue = all_issues[0]
    subprocess.run(
        ["jit", "issue", "claim", top_issue["id"], agent_id],
        cwd=top_issue["repo"]
    )
    
    return top_issue

if __name__ == "__main__":
    agent_id = "agent:multi-repo-worker"
    issue = dispatch_across_repos(agent_id)
    
    if issue:
        print(f"Claimed {issue['id']} from {issue['repo']}")
    else:
        print("No work available")
```

## Best Practices

### 1. Use JSON Output
Always use `--json` flag for machine-readable output:
```bash
jit query ready --json | jq '.issues[]'
```

### 2. Handle Failures Gracefully
```python
try:
    subprocess.run(["jit", "issue", "claim", id, agent], check=True)
except subprocess.CalledProcessError:
    # Issue already claimed, continue
    pass
```

### 3. Respect Capacity Limits
Track how many issues each agent has claimed:
```python
def get_agent_load(agent_id):
    result = subprocess.run(
        ["jit", "query", "assignee", agent_id, "--json"],
        capture_output=True, text=True
    )
    return len(json.loads(result.stdout)["issues"])
```

### 4. Monitor Event Log
Use the event log for audit trail and debugging:
```bash
jit events tail | grep "issue_claimed"
```

### 5. Implement Idempotency
Orchestrators should be safe to run multiple times:
```python
def safe_dispatch():
    # Check if already assigned
    if is_already_assigned(issue_id):
        return
    
    # Attempt to claim
    claim_issue(issue_id, agent_id)
```

## Testing Your Orchestrator

### Unit Test with Mock subprocess
```python
from unittest.mock import patch

def test_claim_issue():
    with patch('subprocess.run') as mock_run:
        mock_run.return_value.returncode = 0
        
        orchestrator = SimpleOrchestrator([])
        result = orchestrator.claim_issue("test-id", "agent:test")
        
        assert result == True
        mock_run.assert_called_once()
```

### Integration Test with Real jit
```bash
#!/bin/bash
# test-orchestrator.sh

# Setup test repo
jit init
jit issue create -t "Test task" -p high

# Run orchestrator once
timeout 5s ./my-orchestrator.sh once

# Verify issue was claimed
ASSIGNED=$(jit query assignee "agent:test" --json | jq '.count')
assert_equals "$ASSIGNED" "1"
```

## See Also

- `crates/dispatch/README.md` - Built-in orchestrator documentation
- `docs/design.md` - Architecture and design decisions
- `../../docs/tutorials/` - Complete workflow examples
