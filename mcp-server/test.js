#!/usr/bin/env node
/**
 * Test suite for JIT MCP Server
 * 
 * Tests the MCP protocol implementation by sending JSON-RPC requests
 * and validating responses.
 */

import { spawn } from 'child_process';
import { readFileSync, mkdirSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

const TIMEOUT = 5000; // 5 seconds per test

class MCPTester {
  constructor() {
    this.server = null;
    this.responseBuffer = '';
    this.pendingRequests = new Map();
    this.nextId = 1;
    this.testDir = null;
  }

  async start() {
    // Create temporary test directory
    this.testDir = join(tmpdir(), `jit-mcp-test-${Date.now()}`);
    mkdirSync(this.testDir, { recursive: true });
    
    // Get absolute path to index.js (in mcp-server directory)
    const serverPath = join(process.cwd(), 'index.js');
    
    return new Promise((resolve, reject) => {
      this.server = spawn('node', [serverPath], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: this.testDir  // Run in test directory
      });

      this.server.stdout.on('data', (data) => {
        this.responseBuffer += data.toString();
        this.processResponses();
      });

      this.server.stderr.on('data', (data) => {
        console.error('Server stderr:', data.toString());
      });

      this.server.on('error', (err) => {
        reject(err);
      });

      // Give server time to start
      setTimeout(resolve, 100);
    });
  }

  processResponses() {
    const lines = this.responseBuffer.split('\n');
    this.responseBuffer = lines.pop() || ''; // Keep incomplete line

    for (const line of lines) {
      if (!line.trim()) continue;
      
      try {
        const response = JSON.parse(line);
        const pending = this.pendingRequests.get(response.id);
        if (pending) {
          pending.resolve(response);
          this.pendingRequests.delete(response.id);
        }
      } catch (err) {
        console.error('Failed to parse response:', line, err);
      }
    }
  }

  async request(method, params = {}) {
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
        reject(new Error(`Request timeout: ${method}`));
      }, TIMEOUT);

      this.pendingRequests.set(id, {
        resolve: (response) => {
          clearTimeout(timeout);
          resolve(response);
        },
        reject
      });

      this.server.stdin.write(JSON.stringify(request) + '\n');
    });
  }

  async stop() {
    if (this.server) {
      this.server.kill();
      this.server = null;
    }
    
    // Clean up test directory
    if (this.testDir) {
      try {
        rmSync(this.testDir, { recursive: true, force: true });
      } catch (err) {
        console.error('Warning: Failed to clean up test directory:', err.message);
      }
    }
  }
}

// Test utilities
function assert(condition, message) {
  if (!condition) {
    throw new Error(`Assertion failed: ${message}`);
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}\n  Expected: ${expected}\n  Actual: ${actual}`);
  }
}

// Test suite
const tests = [
  {
    name: 'Initialize protocol',
    async run(tester) {
      const response = await tester.request('initialize', {
        protocolVersion: '2024-11-05',
        capabilities: {},
        clientInfo: {
          name: 'test-client',
          version: '1.0.0'
        }
      });

      assert(response.result, 'Should have result');
      assert(response.result.protocolVersion, 'Should return protocol version');
      assert(response.result.serverInfo, 'Should return server info');
      assertEqual(response.result.serverInfo.name, 'jit-mcp-server', 'Server name should match');
    }
  },

  {
    name: 'List tools',
    async run(tester) {
      const response = await tester.request('tools/list');

      assert(response.result, 'Should have result');
      assert(response.result.tools, 'Should have tools array');
      assert(response.result.tools.length > 0, 'Should have at least one tool');

      // Check for key tools
      const toolNames = response.result.tools.map(t => t.name);
      assert(toolNames.includes('jit_issue_create'), 'Should have jit_issue_create tool');
      assert(toolNames.includes('jit_issue_list'), 'Should have jit_issue_list tool');
      assert(toolNames.includes('jit_status'), 'Should have jit_status tool');

      console.log(`  ✓ Found ${response.result.tools.length} tools`);
    }
  },

  {
    name: 'Validate tool schemas have new states',
    async run(tester) {
      const response = await tester.request('tools/list');
      
      // Load schema to verify
      const schema = JSON.parse(readFileSync('./jit-schema.json', 'utf8'));
      
      assert(schema.types.State, 'Schema should have State type');
      assert(schema.types.State.enum, 'State should have enum values');
      
      const states = schema.types.State.enum;
      assert(states.includes('backlog'), 'State enum should include backlog');
      assert(states.includes('gated'), 'State enum should include gated');
      assert(!states.includes('open'), 'State enum should not include open');
      
      console.log(`  ✓ State enum: ${states.join(', ')}`);
    }
  },

  {
    name: 'Initialize test repository',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_init',
        arguments: {}
      });

      assert(response.result, 'Should have result');
      assert(response.result.content, 'Should have content');
      
      const content = response.result.content[0];
      // Either success or already initialized
      assert(content.type === 'text', 'Content should be text');
      
      console.log(`  ✓ Repository initialized`);
    }
  },

  {
    name: 'Call tool - jit_status',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_status',
        arguments: {}
      });

      assert(response.result, 'Should have result');
      assert(response.result.content, 'Should have content');
      assert(response.result.content.length > 0, 'Should have content items');
      
      const content = response.result.content[0];
      assert(content.type === 'text', 'Content should be text');
      
      // MCP server unwraps the {success, data} wrapper
      const output = JSON.parse(content.text);
      assert(typeof output.total === 'number', 'Should have total count');
      
      console.log(`  ✓ Status: ${output.total} total issues`);
    }
  },

  {
    name: 'Tool call with invalid arguments',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_issue_create',
        arguments: {
          // Missing required 'title' argument
          priority: 'high'
        }
      });

      // Should return error in content, not throw
      assert(response.result, 'Should have result');
      assert(response.result.content, 'Should have content');
      
      const content = response.result.content[0].text;
      assert(content.includes('Error') || content.includes('error'), 
        'Should contain error message');
      
      console.log(`  ✓ Error handling works correctly`);
    }
  },

  {
    name: 'Invalid tool name',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_nonexistent_tool',
        arguments: {}
      });

      // Should return error in structured format
      assert(response.error || 
        (response.result && response.result.isError),
        'Should return error for invalid tool');
      
      if (response.result && response.result.isError) {
        const content = response.result.content[0].text;
        const parsed = JSON.parse(content);
        assert(parsed.success === false, 'Should have success: false');
        assert(parsed.error, 'Should have error object');
      }
      
      console.log(`  ✓ Invalid tool rejection works`);
    }
  },

  {
    name: 'Search tool exists',
    async run(tester) {
      const response = await tester.request('tools/list', {});
      
      assert(response.result, 'Should have result');
      assert(response.result.tools, 'Should have tools array');
      
      const searchTool = response.result.tools.find(t => t.name === 'jit_search');
      assert(searchTool, 'Should have jit_search tool');
      assert(searchTool.description.includes('Search'), 'Should have search description');
      assert(searchTool.inputSchema, 'Should have input schema');
      assert(searchTool.inputSchema.properties.query, 'Should have query parameter');
      
      console.log(`  ✓ Search tool registered correctly`);
    }
  },

  {
    name: 'Search tool basic query',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_search',
        arguments: {
          query: 'priority'
        }
      });

      assert(response.result, 'Should have result');
      assert(response.result.content, 'Should have content');
      const content = response.result.content[0];
      assert(content.type === 'text', 'Content should be text');
      
      // Parse the output
      const output = JSON.parse(content.text);
      
      // Check if it's a ripgrep-not-installed error
      const outputStr = JSON.stringify(output).toLowerCase();
      if (outputStr.includes('ripgrep')) {
        console.log(`  ⚠ Search skipped (ripgrep not installed)`);
        return;
      }
      
      // If ripgrep is installed, validate the response structure
      assert(typeof output === 'object', 'Should return object');
      
      console.log(`  ✓ Search returned results`);
    }
  },

  {
    name: 'Search with regex flag',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_search',
        arguments: {
          query: 'p[ri]+ority',
          regex: true
        }
      });

      assert(response.result, 'Should have result');
      const text = response.result.content[0].text;
      
      // Check if it's specifically a ripgrep-not-installed error
      if (text.includes('ripgrep') && text.includes('not installed')) {
        console.log(`  ⚠ Regex search skipped (ripgrep not installed)`);
        return;
      }
      
      const output = JSON.parse(text);
      assert(Array.isArray(output.results), 'Should have results');
      
      console.log(`  ✓ Regex search works`);
    }
  },

  {
    name: 'Search with glob filter',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_search',
        arguments: {
          query: 'priority',
          glob: '*.json'
        }
      });

      assert(response.result, 'Should have result');
      const text = response.result.content[0].text;
      
      // Check if it's specifically a ripgrep-not-installed error
      if (text.includes('ripgrep') && text.includes('not installed')) {
        console.log(`  ⚠ Glob search skipped (ripgrep not installed)`);
        return;
      }
      
      const output = JSON.parse(text);
      assert(Array.isArray(output.results), 'Should have results');
      
      // All results should be from .json files (if any results)
      for (const result of output.results) {
        assert(result.path.endsWith('.json'), 'Should only match .json files');
      }
      
      console.log(`  ✓ Glob filter works (${output.total} JSON matches)`);
    }
  },

  {
    name: 'Nested subcommand tool exists (doc.assets.list)',
    async run(tester) {
      const response = await tester.request('tools/list', {});
      
      assert(response.result, 'Should have result');
      assert(response.result.tools, 'Should have tools array');
      
      const assetsTool = response.result.tools.find(t => t.name === 'jit_doc_assets_list');
      assert(assetsTool, 'Should have jit_doc_assets_list tool');
      assert(assetsTool.description.includes('asset'), 'Should have assets description');
      assert(assetsTool.inputSchema, 'Should have input schema');
      assert(assetsTool.inputSchema.properties.id, 'Should have id parameter');
      assert(assetsTool.inputSchema.properties.path, 'Should have path parameter');
      
      console.log(`  ✓ Nested subcommand tool registered`);
    }
  },

  {
    name: 'Nested subcommand CLI mapping',
    async run(tester) {
      // Create a test directory and initialize jit
      const response = await tester.request('tools/call', {
        name: 'jit_init',
        arguments: {}
      });
      
      assert(response.result, 'Should have result');
      
      // Try calling nested subcommand (will fail with no document, but tests mapping)
      const assetsResponse = await tester.request('tools/call', {
        name: 'jit_doc_assets_list',
        arguments: {
          id: 'TESTID',
          path: 'test.md'
        }
      });
      
      assert(assetsResponse.result, 'Should have result');
      
      // The command should execute but might fail with "issue not found" or similar
      // We're just testing that the CLI mapping is correct
      const content = assetsResponse.result.content[0].text;
      
      // Should be JSON response (either success or error)
      const parsed = JSON.parse(content);
      assert(parsed !== undefined, 'Should return JSON');
      
      console.log(`  ✓ Nested CLI mapping works (got response from jit doc assets list)`);
    }
  },

  {
    name: 'Argument validation with required fields',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_doc_assets_list',
        arguments: {
          // Missing required 'id' and 'path'
        }
      });
      
      assert(response.result, 'Should have result');
      assert(response.result.isError, 'Should be marked as error');
      
      const content = response.result.content[0].text;
      const parsed = JSON.parse(content);
      
      assert(parsed.success === false, 'Should indicate failure');
      assert(parsed.error, 'Should have error object');
      assert(parsed.error.code === 'VALIDATION_ERROR', 'Should be validation error');
      assert(parsed.error.message.includes('Required'), 'Should mention required fields');
      
      console.log(`  ✓ Validation rejects missing required fields`);
    }
  },

  {
    name: 'Argument validation with wrong types',
    async run(tester) {
      const response = await tester.request('tools/call', {
        name: 'jit_issue_create',
        arguments: {
          title: 'Test Issue',
          priority: 123  // Should be string, not number
        }
      });
      
      assert(response.result, 'Should have result');
      
      const content = response.result.content[0].text;
      const parsed = JSON.parse(content);
      
      // Either validation error or execution error
      assert(parsed.success === false || parsed.error, 'Should fail with type mismatch');
      
      console.log(`  ✓ Validation handles type mismatches`);
    }
  },

  {
    name: 'Structured error response',
    async run(tester) {
      // Try to call a command that will definitely fail
      const response = await tester.request('tools/call', {
        name: 'jit_issue_show',
        arguments: {
          id: 'NONEXISTENT_ID_12345'
        }
      });
      
      assert(response.result, 'Should have result');
      assert(response.result.isError, 'Should be marked as error');
      
      const content = response.result.content[0].text;
      const parsed = JSON.parse(content);
      
      assert(parsed.success === false, 'Should have success: false');
      assert(parsed.error, 'Should have error object');
      assert(parsed.error.code, 'Should have error code');
      assert(parsed.error.message, 'Should have error message');
      
      console.log(`  ✓ Structured error envelope: ${parsed.error.code}`);
    }
  }
];

// Run tests
async function runTests() {
  console.log('🧪 JIT MCP Server Test Suite\n');
  
  const tester = new MCPTester();
  let passed = 0;
  let failed = 0;

  try {
    console.log('Starting MCP server...');
    await tester.start();
    console.log('✓ Server started\n');

    for (const test of tests) {
      try {
        process.stdout.write(`Testing: ${test.name}... `);
        await test.run(tester);
        console.log('✓');
        passed++;
      } catch (err) {
        console.log('✗');
        console.error(`  Error: ${err.message}`);
        failed++;
      }
    }

    console.log(`\n📊 Results: ${passed} passed, ${failed} failed`);
    
    if (failed > 0) {
      process.exit(1);
    }
  } finally {
    await tester.stop();
  }
}

// Handle errors
process.on('unhandledRejection', (err) => {
  console.error('Unhandled rejection:', err);
  process.exit(1);
});

// Run
runTests().catch(err => {
  console.error('Test suite failed:', err);
  process.exit(1);
});
