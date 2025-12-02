#!/usr/bin/env python3
"""
Simple Python orchestrator for jit issue tracker.

Demonstrates:
- Querying ready issues
- Priority-based dispatch
- Agent pool management
- Capacity tracking
"""

import json
import subprocess
import time
from dataclasses import dataclass
from typing import List, Optional


@dataclass
class Agent:
    """Agent configuration"""
    id: str
    max_concurrent: int
    current_load: int = 0


class SimpleOrchestrator:
    """Minimal orchestrator implementation"""
    
    def __init__(self, agents: List[Agent], poll_interval: int = 30):
        self.agents = agents
        self.poll_interval = poll_interval
    
    def query_ready_issues(self) -> List[dict]:
        """Query jit for ready issues"""
        try:
            result = subprocess.run(
                ["jit", "query", "ready", "--json"],
                capture_output=True,
                text=True,
                check=True
            )
            data = json.loads(result.stdout)
            return data.get("issues", [])
        except (subprocess.CalledProcessError, json.JSONDecodeError) as e:
            print(f"Error querying ready issues: {e}")
            return []
    
    def claim_issue(self, issue_id: str, agent_id: str) -> bool:
        """Claim an issue for an agent"""
        try:
            subprocess.run(
                ["jit", "issue", "claim", issue_id, agent_id],
                capture_output=True,
                check=True
            )
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to claim {issue_id} for {agent_id}: {e}")
            return False
    
    def get_available_agent(self) -> Optional[Agent]:
        """Find an agent with available capacity"""
        for agent in self.agents:
            if agent.current_load < agent.max_concurrent:
                return agent
        return None
    
    def dispatch_cycle(self) -> int:
        """Run one dispatch cycle, return number of assignments made"""
        ready_issues = self.query_ready_issues()
        
        if not ready_issues:
            return 0
        
        # Sort by priority (critical > high > normal > low)
        priority_order = {"critical": 0, "high": 1, "normal": 2, "low": 3}
        ready_issues.sort(
            key=lambda i: priority_order.get(i.get("priority", "normal"), 4)
        )
        
        assignments = 0
        
        for issue in ready_issues:
            agent = self.get_available_agent()
            if not agent:
                break  # No more capacity
            
            issue_id = issue["id"]
            agent_id_full = f"agent:{agent.id}"
            
            if self.claim_issue(issue_id, agent_id_full):
                print(f"Assigned {issue_id} to {agent.id}")
                agent.current_load += 1
                assignments += 1
        
        return assignments
    
    def run_daemon(self):
        """Run orchestrator in daemon mode"""
        print(f"Starting orchestrator with {len(self.agents)} agents")
        print(f"Poll interval: {self.poll_interval}s")
        
        try:
            while True:
                assigned = self.dispatch_cycle()
                if assigned > 0:
                    print(f"Assigned {assigned} issue(s)")
                
                time.sleep(self.poll_interval)
        except KeyboardInterrupt:
            print("\nShutting down orchestrator")


def main():
    """Example usage"""
    # Define agent pool
    agents = [
        Agent(id="copilot-1", max_concurrent=3),
        Agent(id="copilot-2", max_concurrent=3),
        Agent(id="ci-runner-1", max_concurrent=5),
    ]
    
    # Create and run orchestrator
    orchestrator = SimpleOrchestrator(agents, poll_interval=30)
    orchestrator.run_daemon()


if __name__ == "__main__":
    main()
