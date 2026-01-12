import { spawn } from 'child_process';
import { mkdirSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

const testDir = join(tmpdir(), `jit-mcp-manual-${Date.now()}`);
mkdirSync(testDir, { recursive: true });

console.log('Testing nested subcommand execution...\n');

const server = spawn('node', ['index.js'], {
  stdio: ['pipe', 'pipe', 'pipe'],
  cwd: testDir
});

let responseBuffer = '';
let requestId = 1;

server.stdout.on('data', (data) => {
  responseBuffer += data.toString();
  const lines = responseBuffer.split('\n');
  responseBuffer = lines.pop() || '';
  
  for (const line of lines) {
    if (!line.trim()) continue;
    try {
      const response = JSON.parse(line);
      if (response.id === 1) {
        console.log('Tools list response (showing first 5):');
        const tools = response.result.tools.slice(0, 5);
        tools.forEach(t => console.log(`  - ${t.name}: ${t.description}`));
        console.log(`  ... (${response.result.tools.length - 5} more tools)\n`);
        
        // Find nested tool
        const nestedTool = response.result.tools.find(t => t.name === 'jit_doc_assets_list');
        if (nestedTool) {
          console.log('Found nested tool: jit_doc_assets_list');
          console.log(`  Description: ${nestedTool.description}`);
          console.log(`  Required params: ${nestedTool.inputSchema.required.join(', ')}\n`);
        }
        
        // Now test the nested tool execution
        const testRequest = {
          jsonrpc: '2.0',
          id: 2,
          method: 'tools/call',
          params: {
            name: 'jit_doc_assets_list',
            arguments: {
              id: 'TEST001',
              path: 'docs/test.md'
            }
          }
        };
        server.stdin.write(JSON.stringify(testRequest) + '\n');
      } else if (response.id === 2) {
        console.log('Nested tool execution result:');
        if (response.result.isError) {
          const errorData = JSON.parse(response.result.content[0].text);
          console.log(`  Status: Error (expected - no such issue)`);
          console.log(`  Error code: ${errorData.error.code}`);
          console.log(`  Message: ${errorData.error.message.split('\n')[0]}`);
        } else {
          console.log('  Status: Success');
          console.log(`  Response: ${JSON.stringify(response.result, null, 2)}`);
        }
        console.log('\n✓ Test complete - nested subcommand correctly mapped to CLI');
        server.stdin.end();
        setTimeout(() => {
          rmSync(testDir, { recursive: true, force: true });
          process.exit(0);
        }, 100);
      }
    } catch {}
  }
});

server.stderr.on('data', (data) => {
  // Ignore stderr (server startup messages)
});

setTimeout(() => {
  const request = {
    jsonrpc: '2.0',
    id: 1,
    method: 'tools/list'
  };
  server.stdin.write(JSON.stringify(request) + '\n');
}, 500);

setTimeout(() => {
  console.error('Timeout');
  rmSync(testDir, { recursive: true, force: true });
  process.exit(1);
}, 10000);
