#!/bin/bash
# Minimal bash one-liner orchestrator
#
# This demonstrates how simple it is to build a custom orchestrator using jit CLI.
# Run this in a jit repository directory.

set -e

AGENT_ID="agent:bash-worker-1"
POLL_INTERVAL=5

echo "Starting bash orchestrator (agent: $AGENT_ID, poll: ${POLL_INTERVAL}s)"

while true; do
    # Query for ready issues
    READY_JSON=$(jit query ready --json 2>/dev/null || echo '{"issues":[]}')
    
    # Extract first issue ID using jq (or simple parsing)
    if command -v jq &> /dev/null; then
        ISSUE_ID=$(echo "$READY_JSON" | jq -r '.issues[0].id // empty')
    else
        # Fallback: simple grep parsing (less robust)
        ISSUE_ID=$(echo "$READY_JSON" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)
    fi
    
    if [ -n "$ISSUE_ID" ]; then
        echo "$(date '+%Y-%m-%d %H:%M:%S') - Found ready issue: $ISSUE_ID"
        
        # Claim the issue
        if jit issue claim "$ISSUE_ID" "$AGENT_ID" 2>/dev/null; then
            echo "$(date '+%Y-%m-%d %H:%M:%S') - Claimed $ISSUE_ID for $AGENT_ID"
            
            # Simulate work (replace with actual agent logic)
            echo "$(date '+%Y-%m-%d %H:%M:%S') - Processing $ISSUE_ID..."
            sleep 2
            
            # Mark as done
            jit issue update "$ISSUE_ID" --state done
            echo "$(date '+%Y-%m-%d %H:%M:%S') - Completed $ISSUE_ID"
        else
            echo "$(date '+%Y-%m-%d %H:%M:%S') - Failed to claim $ISSUE_ID (already claimed?)"
        fi
    fi
    
    sleep "$POLL_INTERVAL"
done
