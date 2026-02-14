#!/usr/bin/env python3
"""
Migrate all existing issues to include created_at and updated_at timestamps.

This script:
1. Reads events.jsonl to extract timestamps from event log
2. Falls back to file modification time if no events found
3. Updates all issue JSON files with timestamps
4. Preserves all other issue data unchanged

Usage: python3 scripts/migrate_timestamps.py [--dry-run]
"""

import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Optional, Dict, Tuple
import os

# RFC 3339 format for timestamps
def to_rfc3339(dt: datetime) -> str:
    """Convert datetime to RFC 3339 format."""
    return dt.astimezone().isoformat()

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

def find_timestamps_from_events(issue_id: str, events: list) -> Tuple[Optional[str], Optional[str]]:
    """
    Find created_at and updated_at from event log for given issue.
    
    Returns:
        (created_at, updated_at) tuple or (None, None) if not found
    """
    created_at = None
    updated_at = None
    
    # Filter events for this issue
    issue_events = [e for e in events if e.get('issue_id') == issue_id]
    
    if not issue_events:
        return None, None
    
    # Find issue_created event for created_at
    for event in issue_events:
        if event.get('event_type') == 'issue_created':
            created_at = event.get('timestamp')
            break
    
    # Most recent event timestamp for updated_at
    # Events are in chronological order (append-only log)
    if issue_events:
        updated_at = issue_events[-1].get('timestamp')
    
    return created_at, updated_at

def get_file_mtime_rfc3339(file_path: Path) -> str:
    """Get file modification time as RFC 3339 timestamp."""
    mtime = os.path.getmtime(file_path)
    dt = datetime.fromtimestamp(mtime)
    return to_rfc3339(dt)

def migrate_issue(issue_file: Path, events: list, dry_run: bool = False) -> bool:
    """
    Migrate a single issue file to include timestamps.
    
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
    
    # Check if already has timestamps
    if 'created_at' in issue and 'updated_at' in issue:
        return False
    
    # Try to get timestamps from events
    created_at, updated_at = find_timestamps_from_events(issue_id, events)
    
    # Fallback to file mtime
    file_timestamp = get_file_mtime_rfc3339(issue_file)
    
    if not created_at:
        created_at = file_timestamp
        print(f"  {issue['id'][:8]}: Using file mtime for created_at")
    
    if not updated_at:
        updated_at = file_timestamp
    
    # Add timestamps
    issue['created_at'] = created_at
    issue['updated_at'] = updated_at
    
    if dry_run:
        print(f"  {issue['id'][:8]}: Would add timestamps")
        print(f"    created_at: {created_at}")
        print(f"    updated_at: {updated_at}")
        return True
    
    # Write back
    with open(issue_file, 'w') as f:
        json.dump(issue, f, indent=2)
        f.write('\n')  # Trailing newline for consistency
    
    print(f"  {issue['id'][:8]}: ✓ Added timestamps")
    return True

def main():
    dry_run = '--dry-run' in sys.argv
    
    # Find repository root
    script_dir = Path(__file__).parent
    repo_root = script_dir.parent
    jit_dir = repo_root / '.jit'
    
    if not jit_dir.exists():
        print("Error: .jit directory not found", file=sys.stderr)
        print(f"Expected at: {jit_dir}", file=sys.stderr)
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
        if migrate_issue(issue_file, events, dry_run):
            modified_count += 1
    
    print(f"\n{'Would modify' if dry_run else 'Modified'} {modified_count}/{len(issue_files)} issues")
    
    if dry_run:
        print("\nRun without --dry-run to apply changes")
    else:
        print("\n✅ Migration complete!")
        print("\nNext steps:")
        print("1. Test with: jit query all")
        print("2. Check web UI to verify dates display correctly")
        print("3. Commit changes: git add .jit && git commit -m 'Add timestamps to all issues'")

if __name__ == '__main__':
    main()
