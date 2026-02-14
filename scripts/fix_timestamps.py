#!/usr/bin/env python3
"""
Fix timestamps in all issues by correctly reading from event log.

This script fixes the bug where 'event_type' was used instead of 'type'.
"""

import json
import sys
from pathlib import Path
from datetime import datetime

def load_events(events_file: Path) -> list:
    """Load all events from events.jsonl."""
    events = []
    if not events_file.exists():
        return events
    
    with open(events_file, 'r') as f:
        for line in f:
            line = line.strip()
            if line:
                try:
                    events.append(json.loads(line))
                except json.JSONDecodeError as e:
                    print(f"Warning: Failed to parse event: {e}", file=sys.stderr)
    return events

def fix_issue_timestamps(issue_file: Path, events: list, dry_run: bool = False) -> bool:
    """
    Fix timestamps for a single issue by reading from event log.
    
    Returns:
        True if issue was modified, False otherwise
    """
    # Load issue
    with open(issue_file, 'r') as f:
        issue = json.load(f)
    
    issue_id = issue.get('id')
    if not issue_id:
        print(f"Warning: Issue {issue_file} has no ID, skipping", file=sys.stderr)
        return False
    
    # Get current timestamps
    old_created = issue.get('created_at')
    old_updated = issue.get('updated_at')
    
    if not old_created or not old_updated:
        print(f"  {issue_id[:8]}: No timestamps, skipping")
        return False
    
    # Find all events for this issue
    issue_events = [e for e in events if e.get('issue_id') == issue_id]
    
    if not issue_events:
        print(f"  {issue_id[:8]}: No events found, keeping current timestamps")
        return False
    
    # Get timestamps from events (first and last)
    first_timestamp = issue_events[0].get('timestamp')
    last_timestamp = issue_events[-1].get('timestamp')
    
    if not first_timestamp or not last_timestamp:
        print(f"  {issue_id[:8]}: Events missing timestamps, skipping")
        return False
    
    # Check if we need to update
    if old_created == first_timestamp and old_updated == last_timestamp:
        return False  # Already correct
    
    if dry_run:
        print(f"  {issue_id[:8]}: Would update timestamps")
        print(f"    created_at: {first_timestamp} (was {old_created})")
        print(f"    updated_at: {last_timestamp} (was {old_updated})")
        return True
    
    # Update timestamps
    issue['created_at'] = first_timestamp
    issue['updated_at'] = last_timestamp
    
    # Write back
    with open(issue_file, 'w') as f:
        json.dump(issue, f, indent=2)
        f.write('\n')
    
    print(f"  {issue_id[:8]}: ✓ Fixed timestamps")
    print(f"    created_at: {first_timestamp}")
    print(f"    updated_at: {last_timestamp}")
    return True

def main():
    dry_run = '--dry-run' in sys.argv
    
    # Find repository root
    script_dir = Path(__file__).parent
    repo_root = script_dir.parent
    jit_dir = repo_root / '.jit'
    
    if not jit_dir.exists():
        print("Error: .jit directory not found", file=sys.stderr)
        sys.exit(1)
    
    issues_dir = jit_dir / 'issues'
    events_file = jit_dir / 'events.jsonl'
    
    if not issues_dir.exists():
        print("Error: .jit/issues directory not found", file=sys.stderr)
        sys.exit(1)
    
    print(f"Loading events from {events_file}...")
    events = load_events(events_file)
    print(f"Loaded {len(events)} events")
    
    print(f"\nScanning issues in {issues_dir}...")
    issue_files = list(issues_dir.glob('*.json'))
    print(f"Found {len(issue_files)} issue files")
    
    if dry_run:
        print("\n🔍 DRY RUN MODE - No files will be modified\n")
    else:
        print("\n⚠️  LIVE MODE - Files will be modified\n")
    
    modified_count = 0
    for issue_file in sorted(issue_files):
        if fix_issue_timestamps(issue_file, events, dry_run):
            modified_count += 1
    
    print(f"\n{'Would fix' if dry_run else 'Fixed'} {modified_count}/{len(issue_files)} issues")
    
    if dry_run:
        print("\nRun without --dry-run to apply changes")
    else:
        print("\n✅ Fix complete!")

if __name__ == '__main__':
    main()
