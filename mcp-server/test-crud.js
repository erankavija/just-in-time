#!/usr/bin/env node
/**
 * Test suite for CRUD operations (Phase 1)
 * 
 * Tests comprehensive coverage for:
 * - jit_issue_create (all parameters)
 * - jit_issue_update (all flags including --add-gate/--remove-gate)
 * - jit_issue_show
 * - jit_issue_delete
 * - State transitions and persistence
 */

import { spawn } from 'child_process';
import { mkdirSync, rmSync } from 'fs';
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
    this.testDir = join(tmpdir(), `jit-mcp-crud-test-${Date.now()}`);
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

      this.server.stderr.on('data', (data) => {
        // Ignore stderr unless debugging
      });

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

// Test utilities
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
    if (error.stack && process.env.DEBUG) {
      console.error(error.stack.split('\n').slice(1, 4).join('\n'));
    }
    return false;
  }
}

// Tests
async function testIssueCreateBasic(tester) {
  await tester.callTool('jit_init', {});

  // Initialize first
  await tester.callTool('jit_init', {});
  
  const result = await tester.callTool('jit_issue_create', {
    title: 'Test basic issue'
  });
  
  assert(result.id, 'Should return issue ID');
  assert(result.title === 'Test basic issue', 'Should have correct title');
  assert(result.state, 'Should have state');
}

async function testIssueCreateWithDescription(tester) {
  await tester.callTool('jit_init', {});

  const result = await tester.callTool('jit_issue_create', {
    title: 'Issue with description',
    description: 'This is a detailed description\nwith multiple lines'
  });
  
  assert(result.id, 'Should return issue ID');
  assert(result.description.includes('detailed description'), 'Should have description');
}

async function testIssueCreateWithPriority(tester) {
  await tester.callTool('jit_init', {});

  const result = await tester.callTool('jit_issue_create', {
    title: 'High priority issue',
    priority: 'critical'
  });
  
  assert(result.priority === 'critical', 'Should have correct priority');
}

async function testIssueCreateWithLabels(tester) {
  await tester.callTool('jit_init', {});

  const result = await tester.callTool('jit_issue_create', {
    title: 'Issue with labels',
    label: ['type:task', 'component:backend', 'milestone:v1.0']
  });
  
  assert(result.labels, 'Should have labels');
  assert(result.labels.includes('type:task'), 'Should include type:task');
  assert(result.labels.includes('component:backend'), 'Should include component:backend');
  assert(result.labels.includes('milestone:v1.0'), 'Should include milestone:v1.0');
}

async function testIssueCreateWithGates(tester) {
  await tester.callTool('jit_init', {});

  // First define some gates
  await tester.callTool('jit_gate_define', {
    key: 'test-gate',
    title: 'Test Gate',
    description: 'A test gate'
  });
  
  const result = await tester.callTool('jit_issue_create', {
    title: 'Issue with gates',
    gate: ['test-gate']
  });
  
  assert(result.gates_required, 'Should have gates_required');
  assert(result.gates_required.includes('test-gate'), 'Should include test-gate');
}

async function testIssueShow(tester) {
  await tester.callTool('jit_init', {});

  // Create an issue first
  const created = await tester.callTool('jit_issue_create', {
    title: 'Issue to show',
    description: 'Test description',
    priority: 'high',
    label: ['type:bug']
  });
  
  // Show it
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  
  assert(shown.id === created.id, 'Should return same ID');
  assert(shown.title === 'Issue to show', 'Should have correct title');
  assert(shown.description.includes('Test description'), 'Should have description');
  assert(shown.priority === 'high', 'Should have priority');
  assert(shown.labels.includes('type:bug'), 'Should have label');
}

async function testIssueUpdateTitle(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Original title'
  });
  
  const updated = await tester.callTool('jit_issue_update', {
    id: created.id,
    title: 'Updated title'
  });
  
  assert(updated.title === 'Updated title', 'Should have updated title');
  
  // Verify persistence
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.title === 'Updated title', 'Updated title should persist');
}

async function testIssueUpdateDescription(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Test issue'
  });
  
  const updated = await tester.callTool('jit_issue_update', {
    id: created.id,
    description: 'New description'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.description.includes('New description'), 'Should have new description');
}

async function testIssueUpdateState(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'State test issue'
  });
  
  // Update to ready
  await tester.callTool('jit_issue_update', {
    id: created.id,
    state: 'ready'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.state === 'ready', 'Should be in ready state');
}

async function testIssueUpdatePriority(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Priority test',
    priority: 'low'
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    priority: 'critical'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.priority === 'critical', 'Should have updated priority');
}

async function testIssueUpdateAddLabel(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Label test',
    label: ['type:task']
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    label: ['component:backend']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.labels.includes('type:task'), 'Should keep original label');
  assert(shown.labels.includes('component:backend'), 'Should add new label');
}

async function testIssueUpdateRemoveLabel(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Remove label test',
    label: ['type:task', 'component:backend']
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    'remove-label': ['component:backend']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.labels.includes('type:task'), 'Should keep type:task');
  assert(!shown.labels.includes('component:backend'), 'Should remove component:backend');
}

async function testIssueUpdateAddGate(tester) {
  await tester.callTool('jit_init', {});

  // Define a gate
  await tester.callTool('jit_gate_define', {
    key: 'add-test-gate',
    title: 'Add Test Gate',
    description: 'Gate for testing add'
  });
  
  const created = await tester.callTool('jit_issue_create', {
    title: 'Add gate test'
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    'add-gate': ['add-test-gate']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.gates_required?.includes('add-test-gate'), 'Should have added gate');
}

async function testIssueUpdateRemoveGate(tester) {
  await tester.callTool('jit_init', {});

  // Define a gate
  await tester.callTool('jit_gate_define', {
    key: 'remove-test-gate',
    title: 'Remove Test Gate',
    description: 'Gate for testing remove'
  });
  
  const created = await tester.callTool('jit_issue_create', {
    title: 'Remove gate test',
    gate: ['remove-test-gate']
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    'remove-gate': ['remove-test-gate']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(!shown.gates_required?.includes('remove-test-gate'), 'Should have removed gate');
}

async function testIssueUpdateAssignee(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Assignee test'
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    assignee: 'agent:test-worker'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.assignee === 'agent:test-worker', 'Should have assignee');
}

async function testIssueUpdateUnassign(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Unassign test'
  });
  
  // Assign first
  await tester.callTool('jit_issue_update', {
    id: created.id,
    assignee: 'agent:test-worker'
  });
  
  // Unassign
  await tester.callTool('jit_issue_update', {
    id: created.id,
    unassign: true
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  assert(shown.assignee === null, 'Should have no assignee');
}

async function testIssueDelete(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Issue to delete'
  });
  
  await tester.callTool('jit_issue_delete', {
    id: created.id
  });
  
  // Try to show - should fail
  try {
    await tester.callTool('jit_issue_show', {
      id: created.id
    });
    throw new Error('Should have failed to show deleted issue');
  } catch (err) {
    // Expected to fail
    assert(err.message.includes('not found') || err.message.includes('No content'), 
      'Should fail with not found error');
  }
}

async function testStateTransitionsPersist(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'State persistence test'
  });
  
  const states = ['ready', 'in_progress', 'done'];
  
  for (const state of states) {
    await tester.callTool('jit_issue_update', {
      id: created.id,
      state
    });
    
    const shown = await tester.callTool('jit_issue_show', {
      id: created.id
    });
    assert(shown.state === state, `State should be ${state}`);
  }
}

async function testMultipleFieldsUpdate(tester) {
  await tester.callTool('jit_init', {});

  const created = await tester.callTool('jit_issue_create', {
    title: 'Multi-update test',
    priority: 'low'
  });
  
  await tester.callTool('jit_issue_update', {
    id: created.id,
    title: 'Updated title',
    priority: 'high',
    description: 'Updated description',
    state: 'ready',
    label: ['component:frontend']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: created.id
  });
  
  assert(shown.title === 'Updated title', 'Should update title');
  assert(shown.priority === 'high', 'Should update priority');
  assert(shown.description.includes('Updated description'), 'Should update description');
  assert(shown.state === 'ready', 'Should update state');
  assert(shown.labels.includes('component:frontend'), 'Should add label');
}

// Main test runner
async function main() {
  console.log('Testing CRUD operations (Phase 1)...\n');
  
  const tester = new MCPTester();
  let passed = 0;
  let failed = 0;

  try {
    await tester.start();
    console.log('MCP server started\n');

    // Issue Create tests
    if (await runTest('Issue create - basic', () => testIssueCreateBasic(tester))) passed++; else failed++;
    if (await runTest('Issue create - with description', () => testIssueCreateWithDescription(tester))) passed++; else failed++;
    if (await runTest('Issue create - with priority', () => testIssueCreateWithPriority(tester))) passed++; else failed++;
    if (await runTest('Issue create - with labels', () => testIssueCreateWithLabels(tester))) passed++; else failed++;
    if (await runTest('Issue create - with gates', () => testIssueCreateWithGates(tester))) passed++; else failed++;
    
    // Issue Show tests
    if (await runTest('Issue show', () => testIssueShow(tester))) passed++; else failed++;
    
    // Issue Update tests
    if (await runTest('Issue update - title', () => testIssueUpdateTitle(tester))) passed++; else failed++;
    if (await runTest('Issue update - description', () => testIssueUpdateDescription(tester))) passed++; else failed++;
    if (await runTest('Issue update - state', () => testIssueUpdateState(tester))) passed++; else failed++;
    if (await runTest('Issue update - priority', () => testIssueUpdatePriority(tester))) passed++; else failed++;
    if (await runTest('Issue update - add label', () => testIssueUpdateAddLabel(tester))) passed++; else failed++;
    if (await runTest('Issue update - remove label', () => testIssueUpdateRemoveLabel(tester))) passed++; else failed++;
    if (await runTest('Issue update - add gate', () => testIssueUpdateAddGate(tester))) passed++; else failed++;
    if (await runTest('Issue update - remove gate', () => testIssueUpdateRemoveGate(tester))) passed++; else failed++;
    if (await runTest('Issue update - assignee', () => testIssueUpdateAssignee(tester))) passed++; else failed++;
    if (await runTest('Issue update - unassign', () => testIssueUpdateUnassign(tester))) passed++; else failed++;
    if (await runTest('Issue update - multiple fields', () => testMultipleFieldsUpdate(tester))) passed++; else failed++;
    
    // Issue Delete tests
    if (await runTest('Issue delete', () => testIssueDelete(tester))) passed++; else failed++;
    
    // State persistence
    if (await runTest('State transitions persist', () => testStateTransitionsPersist(tester))) passed++; else failed++;

  } finally {
    await tester.stop();
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch(console.error);
