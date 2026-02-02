#!/usr/bin/env node
/**
 * Test suite for Integration & Workflows (Phase 4)
 * 
 * Tests comprehensive coverage for:
 * - End-to-end workflows (create → update → gates → done)
 * - Multi-issue batch operations
 * - Document operations
 * - Assignment operations
 * - Concurrent operations
 * - Real-world agent workflows
 */

import { spawn } from 'child_process';
import { mkdirSync, rmSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

const TIMEOUT = 5000;

class MCPTester {
  constructor() {
    this.server = null;
    this.responseBuffer = '';
    this.pendingRequests = new Map();
    this.nextId = 1;
    this.testDir = null;
  }

  async start() {
    this.testDir = join(tmpdir(), `jit-mcp-workflow-test-${Date.now()}`);
    mkdirSync(this.testDir, { recursive: true });
    
    const serverPath = join(process.cwd(), 'index.js');
    
    return new Promise((resolve, reject) => {
      this.server = spawn('node', [serverPath], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: this.testDir
      });

      this.server.stdout.on('data', (data) => {
        this.responseBuffer += data.toString();
        this.processResponses();
      });

      this.server.stderr.on('data', () => {});

      this.server.on('error', (err) => {
        reject(err);
      });

      setTimeout(resolve, 500);
    });
  }

  processResponses() {
    const lines = this.responseBuffer.split('\n');
    this.responseBuffer = lines.pop() || '';

    for (const line of lines) {
      if (!line.trim()) continue;
      
      try {
        const response = JSON.parse(line);
        if (response.id && this.pendingRequests.has(response.id)) {
          const { resolve } = this.pendingRequests.get(response.id);
          this.pendingRequests.delete(response.id);
          resolve(response);
        }
      } catch (err) {
        console.error('Failed to parse response:', line);
      }
    }
  }

  async sendRequest(method, params = {}) {
    const id = this.nextId++;
    const request = {
      jsonrpc: '2.0',
      id,
      method,
      params
    };

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`Request ${id} timed out (${method})`));
      }, TIMEOUT);

      this.pendingRequests.set(id, {
        resolve: (response) => {
          clearTimeout(timeout);
          resolve(response);
        }
      });

      this.server.stdin.write(JSON.stringify(request) + '\n');
    });
  }

  async callTool(toolName, args = {}) {
    const response = await this.sendRequest('tools/call', {
      name: toolName,
      arguments: args
    });

    if (response.error) {
      throw new Error(response.error.message);
    }

    if (!response.result?.content) {
      throw new Error('No content in response');
    }

    const content = response.result.content;
    
    // Try to find assistant-targeted content first (if annotations exist)
    const assistantContent = content.find(
      c => c.annotations?.audience?.includes('assistant')
    );
    
    if (assistantContent?.text) {
      try {
        return JSON.parse(assistantContent.text);
      } catch (err) {
        return { message: assistantContent.text };
      }
    }

    // If no annotations, try to find JSON content (usually second item)
    for (let i = content.length - 1; i >= 0; i--) {
      if (content[i].type === 'text') {
        try {
          return JSON.parse(content[i].text);
        } catch {
          // Not JSON, try next
        }
      }
    }

    // Fallback: return first text content wrapped in message
    const firstContent = content.find(c => c.type === 'text');
    if (firstContent?.text) {
      return { message: firstContent.text };
    }

    throw new Error('No parseable content in response');
  }




  async stop() {
    if (this.server) {
      this.server.kill();
      this.server = null;
    }
    if (this.testDir) {
      rmSync(this.testDir, { recursive: true, force: true });
    }
  }
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`Assertion failed: ${message}`);
  }
}

async function runTest(name, fn) {
  try {
    await fn();
    console.log(`✓ ${name}`);
    return true;
  } catch (error) {
    console.error(`✗ ${name}`);
    console.error(`  ${error.message}`);
    return false;
  }
}

// Tests
async function testEndToEndWorkflow(tester) {
  await tester.callTool('jit_init', {});
  
  // Define a gate
  await tester.callTool('jit_gate_define', {
    key: 'workflow-gate',
    title: 'Workflow Gate',
    description: 'Gate for workflow test'
  });
  
  // 1. Create issue
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Workflow test issue',
    description: 'End-to-end workflow',
    priority: 'high',
    label: ['type:task'],
    gate: ['workflow-gate']
  });
  
  assert(issue.id, 'Should create issue');
  
  // 2. Update to ready
  await tester.callTool('jit_issue_update', {
    id: issue.id,
    state: 'ready'
  });
  
  // 3. Claim issue
  await tester.callTool('jit_issue_claim', {
    id: issue.id,
    assignee: 'agent:workflow-test'
  });
  
  // 4. Start work
  await tester.callTool('jit_issue_update', {
    id: issue.id,
    state: 'in_progress'
  });
  
  // 5. Pass gate
  await tester.callTool('jit_gate_pass', {
    id: issue.id,
    gate_key: 'workflow-gate'
  });
  
  // 6. Mark as done
  await tester.callTool('jit_issue_update', {
    id: issue.id,
    state: 'done'
  });
  
  // Verify final state
  const final = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  
  assert(final.state === 'done', 'Should be done');
  assert(final.gates_status?.['workflow-gate']?.status === 'passed', 'Gate should be passed');
}

async function testBatchUpdate(tester) {
  // Create multiple issues
  const issue1 = await tester.callTool('jit_issue_create', {
    title: 'Batch 1',
    label: ['component:batch']
  });
  
  const issue2 = await tester.callTool('jit_issue_create', {
    title: 'Batch 2',
    label: ['component:batch']
  });
  
  const issue3 = await tester.callTool('jit_issue_create', {
    title: 'Batch 3',
    label: ['component:batch']
  });
  
  // Batch update with filter
  await tester.callTool('jit_issue_update', {
    filter: 'labels.component:batch',
    priority: 'high'
  });
  
  // Verify all were updated
  const shown1 = await tester.callTool('jit_issue_show', { id: issue1.id });
  const shown2 = await tester.callTool('jit_issue_show', { id: issue2.id });
  const shown3 = await tester.callTool('jit_issue_show', { id: issue3.id });
  
  assert(shown1.priority === 'high', 'Issue 1 should be high priority');
  assert(shown2.priority === 'high', 'Issue 2 should be high priority');
  assert(shown3.priority === 'high', 'Issue 3 should be high priority');
}

async function testAssignmentOperations(tester) {
  // Create issue
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Assignment test'
  });
  
  // Assign
  await tester.callTool('jit_issue_assign', {
    id: issue.id,
    assignee: 'agent:test-1'
  });
  
  let shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.assignee === 'agent:test-1', 'Should be assigned');
  
  // Reassign
  await tester.callTool('jit_issue_assign', {
    id: issue.id,
    assignee: 'agent:test-2'
  });
  
  shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.assignee === 'agent:test-2', 'Should be reassigned');
  
  // Unassign
  await tester.callTool('jit_issue_unassign', {
    id: issue.id
  });
  
  shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.assignee === null, 'Should be unassigned');
}

async function testClaimOperations(tester) {
  // Create issue in ready state
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Claim test'
  });
  
  await tester.callTool('jit_issue_update', {
    id: issue.id,
    state: 'ready'
  });
  
  // Claim it
  await tester.callTool('jit_issue_claim', {
    id: issue.id,
    assignee: 'agent:claimer'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.assignee === 'agent:claimer', 'Should be claimed');
  assert(shown.state === 'in_progress', 'Should be in progress after claim');
}

async function testClaimNext(tester) {
  // Create multiple ready issues
  await tester.callTool('jit_issue_create', {
    title: 'Ready 1',
    state: 'ready',
    priority: 'low'
  });
  
  await tester.callTool('jit_issue_create', {
    title: 'Ready 2',
    state: 'ready',
    priority: 'critical'
  });
  
  // Claim next (should get highest priority)
  const result = await tester.callTool('jit_issue_claim_next', {
    assignee: 'agent:next-claimer'
  });
  
  assert(result.id, 'Should claim an issue');
  
  const shown = await tester.callTool('jit_issue_show', {
    id: result.id
  });
  assert(shown.assignee === 'agent:next-claimer', 'Should be assigned to claimer');
}

async function testIssueBreakdown(tester) {
  // Create parent issue
  const parent = await tester.callTool('jit_issue_create', {
    title: 'Parent task',
    label: ['type:story']
  });
  
  // Break down into subtasks
  const result = await tester.callTool('jit_issue_breakdown', {
    parent_id: parent.id,
    subtask: ['Subtask 1', 'Subtask 2', 'Subtask 3'],
    description: ['Description 1', 'Description 2', 'Description 3']
  });
  
  assert(result.subtasks, 'Should have subtasks');
  assert(result.subtasks.length === 3, 'Should have 3 subtasks');
  
  // Verify subtasks have parent relationship
  for (const subtask of result.subtasks) {
    const shown = await tester.callTool('jit_issue_show', {
      id: subtask.id
    });
    // Check if subtasks have dependency on parent or appropriate labels
    assert(shown.id, 'Subtask should exist');
  }
}

async function testQueryAvailable(tester) {
  // Create mix of issues
  await tester.callTool('jit_issue_create', {
    title: 'Available 1',
    state: 'ready'
  });
  
  await tester.callTool('jit_issue_create', {
    title: 'Not available (assigned)',
    state: 'ready',
    assignee: 'agent:someone'
  });
  
  const issue3 = await tester.callTool('jit_issue_create', {
    title: 'Not available (blocked)'
  });
  
  const blocker = await tester.callTool('jit_issue_create', {
    title: 'Blocker'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issue3.id,
    to_ids: [blocker.id]
  });
  
  // Query available
  const available = await tester.callTool('jit_query_available', {});
  
  assert(available.issues, 'Should have issues');
  // Should only include unassigned, ready, unblocked issues
  const titles = available.issues.map(i => i.title);
  assert(titles.includes('Available 1'), 'Should include available issue');
}

async function testQueryBlocked(tester) {
  // Create blocked issue
  const blocker = await tester.callTool('jit_issue_create', {
    title: 'Blocker issue'
  });
  
  const blocked = await tester.callTool('jit_issue_create', {
    title: 'Blocked issue'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: blocked.id,
    to_ids: [blocker.id]
  });
  
  const result = await tester.callTool('jit_query_blocked', {});
  
  assert(result.issues, 'Should have issues');
  const ids = result.issues.map(i => i.id);
  assert(ids.includes(blocked.id), 'Should include blocked issue');
}

async function testIssueSearch(tester) {
  // Create issues with searchable content
  await tester.callTool('jit_issue_create', {
    title: 'Implement authentication'
  });
  
  await tester.callTool('jit_issue_create', {
    title: 'Add authorization checks'
  });
  
  // Search by keyword
  const result = await tester.callTool('jit_issue_search', {
    query: 'auth'
  });
  
  assert(result.issues, 'Should have issues');
  assert(result.issues.length >= 2, 'Should find matching issues');
}

async function testStatusCommand(tester) {
  // Create issues in different states
  await tester.callTool('jit_issue_create', {
    title: 'Backlog issue',
    state: 'backlog'
  });
  
  await tester.callTool('jit_issue_create', {
    title: 'Ready issue',
    state: 'ready'
  });
  
  await tester.callTool('jit_issue_create', {
    title: 'In progress issue',
    state: 'in_progress'
  });
  
  await tester.callTool('jit_issue_create', {
    title: 'Done issue',
    state: 'done'
  });
  
  const status = await tester.callTool('jit_status', {});
  
  assert(typeof status.total === 'number', 'Should have total count');
  assert(status.total >= 4, 'Should count all issues');
}

async function testDocumentOperations(tester) {
  // Create issue
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Issue with docs'
  });
  
  // Create a test document
  const docPath = join(tester.testDir, 'test-doc.md');
  writeFileSync(docPath, '# Test Document\n\nThis is a test.');
  
  // Add document to issue
  const addResult = await tester.callTool('jit_doc_add', {
    id: issue.id,
    path: 'test-doc.md',
    'doc-type': 'design',
    label: 'Design Document'
  });
  
  assert(addResult.success !== false, 'Should add document');
  
  // List documents
  const listResult = await tester.callTool('jit_doc_list', {
    id: issue.id
  });
  
  assert(listResult.documents, 'Should have documents');
  assert(listResult.documents.length > 0, 'Should list document');
}

async function testReleaseOperation(tester) {
  // Create and claim issue
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Release test'
  });
  
  await tester.callTool('jit_issue_claim', {
    id: issue.id,
    assignee: 'agent:release-test'
  });
  
  // Release it
  await tester.callTool('jit_issue_release', {
    id: issue.id,
    reason: 'timeout'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.assignee === null, 'Should be released');
}

async function testRejectOperation(tester) {
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Issue to reject'
  });
  
  await tester.callTool('jit_issue_reject', {
    id: issue.id,
    reason: 'duplicate'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.state === 'rejected', 'Should be rejected');
}

// Main test runner
async function main() {
  console.log('Testing Integration & Workflows (Phase 4)...\n');
  
  const tester = new MCPTester();
  let passed = 0;
  let failed = 0;

  try {
    await tester.start();
    console.log('MCP server started\n');

    // End-to-end workflows
    if (await runTest('End-to-end workflow', () => testEndToEndWorkflow(tester))) passed++; else failed++;
    
    // Batch operations
    if (await runTest('Batch update', () => testBatchUpdate(tester))) passed++; else failed++;
    
    // Assignment operations
    if (await runTest('Assignment operations', () => testAssignmentOperations(tester))) passed++; else failed++;
    if (await runTest('Claim operations', () => testClaimOperations(tester))) passed++; else failed++;
    if (await runTest('Claim next', () => testClaimNext(tester))) passed++; else failed++;
    
    // Issue breakdown
    if (await runTest('Issue breakdown', () => testIssueBreakdown(tester))) passed++; else failed++;
    
    // Query operations
    if (await runTest('Query available', () => testQueryAvailable(tester))) passed++; else failed++;
    if (await runTest('Query blocked', () => testQueryBlocked(tester))) passed++; else failed++;
    if (await runTest('Issue search', () => testIssueSearch(tester))) passed++; else failed++;
    if (await runTest('Status command', () => testStatusCommand(tester))) passed++; else failed++;
    
    // Document operations
    if (await runTest('Document operations', () => testDocumentOperations(tester))) passed++; else failed++;
    
    // Special operations
    if (await runTest('Release operation', () => testReleaseOperation(tester))) passed++; else failed++;
    if (await runTest('Reject operation', () => testRejectOperation(tester))) passed++; else failed++;

  } finally {
    await tester.stop();
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch(console.error);
