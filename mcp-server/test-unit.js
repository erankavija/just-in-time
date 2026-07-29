#!/usr/bin/env node
/**
 * Unit tests for MCP server modules
 *
 * Tests the server's own logic — tool generation, input validation,
 * CLI argument building, concurrency limiting — without spawning the
 * CLI or MCP server.
 */

import { strict as assert } from 'node:assert';
import { execFileSync } from 'node:child_process';
import { CURATION, curationCoverage, generateTools, generateDefaultTools, parseToolName, getCommandByPath } from './lib/tool-generator.js';
import { validateArguments, createValidator } from './lib/validator.js';
import { buildCliArgs, getTimeoutForCommand } from './lib/cli-executor.js';
import { ConcurrencyLimiter } from './lib/concurrency.js';

// Load the real schema from the CLI (single source of truth)
const realSchema = JSON.parse(execFileSync('jit', ['--schema'], { maxBuffer: 10 * 1024 * 1024 }).toString());

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

let passed = 0;
let failed = 0;

/**
 * Run `fn` with console.error captured instead of printed, returning the
 * captured messages. Used to assert the $ref resolver's diagnostic calls
 * fire (or stay silent) under specific schema shapes.
 */
function captureConsoleErrors(fn) {
  const messages = [];
  const original = console.error;
  console.error = (...args) => messages.push(args.join(' '));
  try {
    fn();
  } finally {
    console.error = original;
  }
  return messages;
}

async function runTest(name, fn) {
  try {
    await fn();
    console.log(`  \u2713 ${name}`);
    passed++;
  } catch (err) {
    console.log(`  \u2717 ${name}`);
    console.error(`    ${err.message}`);
    failed++;
  }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

// Minimal schema fragment for testing — avoids coupling to full jit-schema.json
const testSchema = {
  version: '0.0.0-test',
  commands: {
    status: {
      description: 'Show overall status',
      args: [],
      flags: [
        { name: 'json', type: 'boolean', required: false, description: 'Output JSON format' },
      ],
      output: {
        success_schema: {
          $schema: 'http://json-schema.org/draft-07/schema#',
          title: 'StatusResponse',
          type: 'object',
          properties: {
            open: { type: 'integer' },
            done: { type: 'integer' },
            total: { type: 'integer' },
          },
          required: ['open', 'done', 'total'],
        },
        success: 'StatusResponse',
        error: 'ErrorResponse',
      },
    },
    issue: {
      description: 'Issue commands',
      subcommands: {
        create: {
          description: 'Create a new issue',
          args: [],
          flags: [
            { name: 'title', type: 'string', required: true, description: 'title flag' },
            { name: 'description', type: 'string', required: false, description: 'description flag' },
            { name: 'priority', type: 'string', required: false, description: 'priority flag' },
            { name: 'label', type: 'array<string>', required: false, description: 'Labels' },
            { name: 'json', type: 'boolean', required: false, description: 'Output JSON format' },
          ],
        },
        show: {
          description: 'Show issue details',
          args: [
            { name: 'id', type: 'string', required: true, description: 'id parameter' },
          ],
          flags: [
            { name: 'json', type: 'boolean', required: false, description: 'Output JSON format' },
          ],
        },
        update: {
          description: 'Update an issue',
          args: [],
          flags: [
            { name: 'id', type: 'string', required: true, description: 'Issue ID' },
            { name: 'title', type: 'string', required: false, description: 'title flag' },
            { name: 'state', type: 'string', required: false, description: 'state flag' },
            { name: 'json', type: 'boolean', required: false, description: 'Output JSON format' },
          ],
        },
      },
    },
    doc: {
      description: 'Document commands',
      subcommands: {
        assets: {
          description: 'Asset commands',
          subcommands: {
            list: {
              description: 'List assets for a document',
              args: [
                { name: 'id', type: 'string', required: true, description: 'Issue ID' },
                { name: 'path', type: 'string', required: true, description: 'Path to document' },
              ],
              flags: [
                { name: 'json', type: 'boolean', required: false, description: 'Output JSON format' },
              ],
            },
          },
        },
      },
    },
    gate: {
      description: 'Gate commands',
      subcommands: {
        define: {
          description: 'Define a new gate',
          hidden: true,
          args: [],
          flags: [
            { name: 'key', type: 'string', required: true, description: 'Unique gate key' },
            { name: 'title', type: 'string', required: true, description: 'Human-readable title' },
            { name: 'description', type: 'string', required: true, description: 'Description' },
            { name: 'mode', type: 'string', required: false, description: 'Gate mode' },
            { name: 'timeout', type: 'number', required: false, description: 'Timeout seconds' },
            { name: 'json', type: 'boolean', required: false, description: 'Output JSON format' },
          ],
        },
      },
    },
  },
};

// ---------------------------------------------------------------------------
// tool-generator tests
// ---------------------------------------------------------------------------

console.log('\ntool-generator.js');

await runTest('generates tools from leaf commands', () => {
  const tools = generateTools(testSchema);
  const names = tools.map(t => t.name);
  assert.ok(names.includes('jit_status'), 'should include jit_status');
  assert.ok(names.includes('jit_issue_create'), 'should include jit_issue_create');
  assert.ok(names.includes('jit_issue_show'), 'should include jit_issue_show');
});

await runTest('does not generate tools for non-leaf commands', () => {
  const tools = generateTools(testSchema);
  const names = tools.map(t => t.name);
  assert.ok(!names.includes('jit_issue'), 'should not include jit_issue (non-leaf)');
  assert.ok(!names.includes('jit_doc'), 'should not include jit_doc (non-leaf)');
  assert.ok(!names.includes('jit_doc_assets'), 'should not include jit_doc_assets (non-leaf)');
});

await runTest('handles deeply nested subcommands', () => {
  const tools = generateTools(testSchema);
  const tool = tools.find(t => t.name === 'jit_doc_assets_list');
  assert.ok(tool, 'should generate jit_doc_assets_list');
  assert.deepStrictEqual(tool._commandPath, ['doc', 'assets', 'list']);
});

await runTest('maps array<string> to JSON Schema array type', () => {
  const tools = generateTools(testSchema);
  const createTool = tools.find(t => t.name === 'jit_issue_create');
  const labelProp = createTool.inputSchema.properties.label;
  assert.strictEqual(labelProp.type, 'array');
  assert.deepStrictEqual(labelProp.items, { type: 'string' });
});

await runTest('propagates required fields to inputSchema', () => {
  const tools = generateTools(testSchema);
  const createTool = tools.find(t => t.name === 'jit_issue_create');
  assert.ok(createTool.inputSchema.required.includes('title'), 'title should be required');
  assert.ok(!createTool.inputSchema.required.includes('description'), 'description should not be required');
});

await runTest('positional args become properties', () => {
  const tools = generateTools(testSchema);
  const showTool = tools.find(t => t.name === 'jit_issue_show');
  assert.ok(showTool.inputSchema.properties.id, 'id should be a property');
  assert.ok(showTool.inputSchema.required.includes('id'), 'id should be required');
});

await runTest('tool description comes from command', () => {
  const tools = generateTools(testSchema);
  const statusTool = tools.find(t => t.name === 'jit_status');
  assert.strictEqual(statusTool.description, 'Show overall status');
});

await runTest('propagates hidden flag to generated tool as _hidden', () => {
  const tools = generateTools(testSchema);
  const defineTool = tools.find(t => t.name === 'jit_gate_define');
  assert.ok(defineTool, 'should generate jit_gate_define');
  assert.strictEqual(defineTool._hidden, true, 'gate define should be _hidden');
  const statusTool = tools.find(t => t.name === 'jit_status');
  assert.strictEqual(statusTool._hidden, false, 'status should not be _hidden');
});

await runTest('parseToolName strips jit_ prefix and splits on _', () => {
  assert.deepStrictEqual(parseToolName('jit_issue_create'), ['issue', 'create']);
  assert.deepStrictEqual(parseToolName('jit_doc_assets_list'), ['doc', 'assets', 'list']);
  assert.deepStrictEqual(parseToolName('jit_status'), ['status']);
});

await runTest('getCommandByPath traverses subcommands', () => {
  const cmd = getCommandByPath(testSchema, ['issue', 'create']);
  assert.ok(cmd, 'should find issue create');
  assert.strictEqual(cmd.description, 'Create a new issue');
});

await runTest('getCommandByPath returns leaf at depth 3', () => {
  const cmd = getCommandByPath(testSchema, ['doc', 'assets', 'list']);
  assert.ok(cmd, 'should find doc assets list');
  assert.strictEqual(cmd.description, 'List assets for a document');
});

await runTest('getCommandByPath returns null for missing command', () => {
  assert.strictEqual(getCommandByPath(testSchema, ['nonexistent']), null);
  assert.strictEqual(getCommandByPath(testSchema, ['issue', 'nonexistent']), null);
});

await runTest('includes outputSchema when success_schema is present', () => {
  const tools = generateTools(testSchema);
  const statusTool = tools.find(t => t.name === 'jit_status');
  assert.ok(statusTool.outputSchema, 'status tool should have outputSchema');
  assert.strictEqual(statusTool.outputSchema.type, 'object');
  assert.ok(statusTool.outputSchema.properties.total, 'should have total property');
  assert.ok(statusTool.outputSchema.required.includes('total'), 'total should be required');
});

await runTest('outputSchema strips $schema and title', () => {
  const tools = generateTools(testSchema);
  const statusTool = tools.find(t => t.name === 'jit_status');
  assert.strictEqual(statusTool.outputSchema.$schema, undefined, 'should strip $schema');
  assert.strictEqual(statusTool.outputSchema.title, undefined, 'should strip title');
});

await runTest('omits outputSchema when success_schema is absent', () => {
  const tools = generateTools(testSchema);
  const createTool = tools.find(t => t.name === 'jit_issue_create');
  assert.strictEqual(createTool.outputSchema, undefined, 'should not have outputSchema');
});

await runTest('real schema: outputSchema resolves $ref definitions inline', () => {

  const tools = generateTools(realSchema);

  // gate.list has $ref to GateDefinition
  const gateListTool = tools.find(t => t.name === 'jit_gate_list');
  if (gateListTool?.outputSchema) {
    const str = JSON.stringify(gateListTool.outputSchema);
    assert.ok(!str.includes('$ref'), 'outputSchema should not contain $ref');
    assert.ok(gateListTool.outputSchema.properties.gates, 'should have gates property');
  }
});

await runTest('real schema: generateTools resolves cleanly with no $ref-resolver warnings', () => {
  const messages = captureConsoleErrors(() => generateTools(realSchema));
  assert.deepStrictEqual(messages, [],
    `$ref resolution should not warn on the real schema, got: ${JSON.stringify(messages)}`);
});

await runTest('outputSchema scopes definitions nested in oneOf branches, not just the outermost bag', () => {
  // Mirrors the real CLI shape: a oneOf response where each branch is an
  // independent JSON Schema document carrying its own local `definitions`,
  // with no definitions at the success_schema's own top level.
  const schema = {
    version: '0.0.0-test',
    commands: {
      thing: {
        description: 'oneOf with branch-local definitions',
        args: [],
        output: {
          success_schema: {
            description: 'one of two shapes',
            oneOf: [
              {
                $schema: 'http://json-schema.org/draft-07/schema#',
                type: 'object',
                definitions: {
                  Widget: { type: 'object', properties: { id: { type: 'string' } }, required: ['id'] },
                },
                properties: { widget: { $ref: '#/definitions/Widget' } },
                required: ['widget'],
              },
              {
                $schema: 'http://json-schema.org/draft-07/schema#',
                type: 'object',
                properties: { empty: { type: 'boolean' } },
                required: ['empty'],
              },
            ],
          },
        },
      },
    },
  };

  const messages = captureConsoleErrors(() => {
    const tools = generateTools(schema);
    const tool = tools.find(t => t.name === 'jit_thing');
    const str = JSON.stringify(tool.outputSchema);
    assert.ok(!str.includes('$ref'), 'branch-local $ref should resolve inline');
    assert.ok(!str.includes('"definitions"'), 'branch-local definitions bag should not leak into output');
    assert.deepStrictEqual(
      tool.outputSchema.oneOf[0].properties.widget,
      { type: 'object', properties: { id: { type: 'string' } }, required: ['id'] });
  });
  assert.deepStrictEqual(messages, [], `should resolve without warnings, got: ${JSON.stringify(messages)}`);
});

await runTest('self-referential (tree-shaped) $ref truncates silently, no circular warning', () => {
  // Mirrors DependencyTreeNode: a node whose children are more of itself.
  // This is a legitimate recursive schema shape, not a generator bug.
  const schema = {
    version: '0.0.0-test',
    commands: {
      tree: {
        description: 'self-referential node',
        args: [],
        output: {
          success_schema: {
            $schema: 'http://json-schema.org/draft-07/schema#',
            type: 'object',
            definitions: {
              Node: {
                type: 'object',
                properties: {
                  id: { type: 'string' },
                  children: { type: 'array', items: { $ref: '#/definitions/Node' } },
                },
                required: ['id', 'children'],
              },
            },
            properties: { root: { $ref: '#/definitions/Node' } },
            required: ['root'],
          },
        },
      },
    },
  };

  const messages = captureConsoleErrors(() => {
    const tools = generateTools(schema);
    const tool = tools.find(t => t.name === 'jit_tree');
    const str = JSON.stringify(tool.outputSchema);
    assert.ok(!str.includes('$ref'), 'recursive $ref should not survive resolution');
    // One real level of Node, truncated to a bare object at the next level.
    const root = tool.outputSchema.properties.root;
    assert.deepStrictEqual(root.properties.id, { type: 'string' });
    assert.deepStrictEqual(root.properties.children.items, { type: 'object' });
  });
  assert.deepStrictEqual(messages, [], `self-recursion should not warn, got: ${JSON.stringify(messages)}`);
});

await runTest('non-productive alias cycle between definitions still warns (diagnostic stays functional)', () => {
  // A bare alias loop (A -> $ref B, B -> $ref A, no wrapping object/array in
  // between) never terminates and can never appear in real generated output;
  // it signals a genuine schema/generator mismatch, so it must still warn.
  const schema = {
    version: '0.0.0-test',
    commands: {
      alias: {
        description: 'degenerate alias cycle',
        args: [],
        output: {
          success_schema: {
            $schema: 'http://json-schema.org/draft-07/schema#',
            type: 'object',
            definitions: {
              A: { $ref: '#/definitions/B' },
              B: { $ref: '#/definitions/A' },
            },
            properties: { value: { $ref: '#/definitions/A' } },
            required: ['value'],
          },
        },
      },
    },
  };

  const messages = captureConsoleErrors(() => generateTools(schema));
  assert.ok(messages.some(m => m.includes('Circular $ref detected: A') || m.includes('Circular $ref detected: B')),
    `non-productive alias cycle should still warn, got: ${JSON.stringify(messages)}`);
});

await runTest('$ref to a genuinely missing definition still warns (diagnostic stays functional)', () => {
  const schema = {
    version: '0.0.0-test',
    commands: {
      missing: {
        description: 'dangling ref',
        args: [],
        output: {
          success_schema: {
            $schema: 'http://json-schema.org/draft-07/schema#',
            type: 'object',
            properties: { value: { $ref: '#/definitions/DoesNotExist' } },
            required: ['value'],
          },
        },
      },
    },
  };

  const messages = captureConsoleErrors(() => generateTools(schema));
  assert.ok(messages.some(m => m.includes('Definition not found: DoesNotExist')),
    `dangling $ref should still warn, got: ${JSON.stringify(messages)}`);
});

await runTest('real schema: tools with success_schema get outputSchema', () => {

  const tools = generateTools(realSchema);
  const withOutput = tools.filter(t => t.outputSchema);
  assert.ok(withOutput.length >= 10, `should have 10+ tools with outputSchema, got ${withOutput.length}`);
  // All must have type: object at root
  for (const tool of withOutput) {
    assert.strictEqual(tool.outputSchema.type, 'object',
      `${tool.name} outputSchema should have type: object`);
  }
});

await runTest('generateTools matches real schema without errors', () => {

  const tools = generateTools(realSchema);
  assert.ok(tools.length > 50, `should generate 50+ tools, got ${tools.length}`);
  // Every tool should have a name, description, and inputSchema
  for (const tool of tools) {
    assert.ok(tool.name.startsWith('jit_'), `tool name should start with jit_: ${tool.name}`);
    assert.ok(tool.description, `tool should have description: ${tool.name}`);
    assert.ok(tool.inputSchema, `tool should have inputSchema: ${tool.name}`);
    assert.strictEqual(tool.inputSchema.type, 'object');
  }
});

await runTest('real schema exposes only dependency-aware archive tools', () => {
  const names = new Set(generateTools(realSchema).map(tool => tool.name));
  const retiredTool = ['jit_doc', 'archive'].join('_');
  assert.ok(!names.has(retiredTool), 'retired document archive tool must not be generated');
  for (const name of ['jit_archive_document', 'jit_archive_container', 'jit_archive_candidates']) {
    assert.ok(names.has(name), `replacement archive tool must remain generated: ${name}`);
  }
});

await runTest('curation manifest decides every generated tool exactly once', () => {

  const { uncurated, stale } = curationCoverage(realSchema);
  assert.deepStrictEqual(uncurated, [],
    `curated-tools.json must include or exclude each generated tool, with a rationale. Undecided: ${uncurated.join(', ')}`);
  assert.deepStrictEqual(stale, [],
    `curated-tools.json names tools the CLI no longer generates: ${stale.join(', ')}`);

  // A rationale is what makes the curation reviewable, so require a real one.
  for (const [name, rationale] of [...Object.entries(CURATION.include), ...Object.entries(CURATION.exclude)]) {
    assert.ok(typeof rationale === 'string' && rationale.length >= 20,
      `${name} needs a one-line rationale in curated-tools.json`);
  }
});

await runTest('generateDefaultTools returns the manifest include set', () => {

  const allTools = generateTools(realSchema);
  const defaultTools = generateDefaultTools(realSchema);
  assert.deepStrictEqual(
    defaultTools.map(t => t.name).sort(),
    Object.keys(CURATION.include).sort(),
    'default tools should be exactly the manifest include set');
  assert.ok(defaultTools.length < allTools.length,
    `curated (${defaultTools.length}) should be fewer than all (${allTools.length})`);
  // Core workflow tools must be present
  for (const name of ['jit_status', 'jit_issue_create', 'jit_issue_show', 'jit_query_available', 'jit_dep_add', 'jit_gate_status-all']) {
    assert.ok(defaultTools.some(t => t.name === name), `missing core tool: ${name}`);
  }
});

await runTest('curated set stays within the manifest headroom bound', () => {

  const defaultTools = generateDefaultTools(realSchema);
  assert.ok(defaultTools.length <= CURATION.max_curated_tools,
    `curated set (${defaultTools.length}) exceeds max_curated_tools (${CURATION.max_curated_tools}); re-curate rather than raising the bound`);
});

await runTest('curation never widens the set beyond what the CLI schema exposes', () => {

  // Commands the CLI marks hidden are excluded here too: the manifest narrows.
  const hiddenIncluded = generateTools(realSchema)
    .filter(t => t._hidden && Object.hasOwn(CURATION.include, t.name))
    .map(t => t.name);
  assert.deepStrictEqual(hiddenIncluded, [],
    `schema-hidden commands must stay excluded: ${hiddenIncluded.join(', ')}`);
});

// ---------------------------------------------------------------------------
// validator tests
// ---------------------------------------------------------------------------

console.log('\nvalidator.js');

await runTest('accepts valid arguments', () => {
  const schema = {
    properties: {
      title: { type: 'string' },
      priority: { type: 'string' },
    },
    required: ['title'],
  };
  const result = validateArguments({ title: 'Test' }, schema);
  assert.ok(result.success);
  assert.strictEqual(result.data.title, 'Test');
});

await runTest('rejects missing required fields and names the offending property', () => {
  const schema = {
    properties: {
      title: { type: 'string' },
    },
    required: ['title'],
  };
  const result = validateArguments({}, schema);
  assert.ok(!result.success);
  assert.strictEqual(result.data, undefined, 'a rejected call must not yield arguments');
  // The caller sends the message straight back to the MCP client, so it has to
  // identify which property was at fault. The wording belongs to the validation
  // library; the property name is this module's contract.
  assert.ok(result.error.includes('title'), `error should name the property: ${result.error}`);
});

await runTest('rejects a non-integer value for an integer-typed property', () => {
  const schema = {
    properties: {
      depth: { type: 'integer' },
    },
    required: ['depth'],
  };
  assert.ok(validateArguments({ depth: 3 }, schema).success);
  const fractional = validateArguments({ depth: 3.5 }, schema);
  assert.ok(!fractional.success, 'integer properties must reject fractional numbers');
  assert.ok(fractional.error.includes('depth'), `error should name the property: ${fractional.error}`);
});

await runTest('accepts any value for a property with no recognised type', () => {
  const schema = {
    properties: {
      payload: { type: 'unmapped-type' },
    },
    required: ['payload'],
  };
  // Unrecognised schema types fall through to an unconstrained validator so a
  // new CLI type never blocks a tool call before it reaches the CLI.
  for (const value of ['text', 42, { nested: true }, ['a']]) {
    const result = validateArguments({ payload: value }, schema);
    assert.ok(result.success, `unmapped type should accept ${JSON.stringify(value)}`);
    assert.deepStrictEqual(result.data.payload, value);
  }
});

await runTest('reports the index path of an offending array item', () => {
  const schema = {
    properties: {
      label: { type: 'array', items: { type: 'string' } },
    },
    required: ['label'],
  };
  const result = validateArguments({ label: ['type:task', 7] }, schema);
  assert.ok(!result.success);
  // Nested failures are reported by their full path so the client can tell
  // which array element was rejected.
  assert.ok(result.error.includes('label.1'), `error should locate the item: ${result.error}`);
});

await runTest('accepts optional fields when missing', () => {
  const schema = {
    properties: {
      title: { type: 'string' },
      description: { type: 'string' },
    },
    required: ['title'],
  };
  const result = validateArguments({ title: 'Test' }, schema);
  assert.ok(result.success);
});

await runTest('validates boolean type', () => {
  const schema = {
    properties: {
      json: { type: 'boolean' },
    },
    required: [],
  };
  const ok = validateArguments({ json: true }, schema);
  assert.ok(ok.success);
  const bad = validateArguments({ json: 'yes' }, schema);
  assert.ok(!bad.success);
});

await runTest('validates number type', () => {
  const schema = {
    properties: {
      timeout: { type: 'number' },
    },
    required: ['timeout'],
  };
  const ok = validateArguments({ timeout: 30 }, schema);
  assert.ok(ok.success);
  const bad = validateArguments({ timeout: 'thirty' }, schema);
  assert.ok(!bad.success);
});

await runTest('validates array type with string items', () => {
  const schema = {
    properties: {
      label: { type: 'array', items: { type: 'string' } },
    },
    required: [],
  };
  const ok = validateArguments({ label: ['a', 'b'] }, schema);
  assert.ok(ok.success);
  const bad = validateArguments({ label: 'not-an-array' }, schema);
  assert.ok(!bad.success);
});

await runTest('a schema default applies to an omitted argument whether or not it is required', () => {
  // tools/list advertises each argument's `default` in the tool inputSchema, so
  // a client that omits the argument must get exactly that value back.
  for (const required of [[], ['mode']]) {
    const schema = {
      properties: {
        mode: { type: 'string', default: 'manual' },
      },
      required,
    };
    const result = validateArguments({}, schema);
    assert.ok(result.success);
    assert.strictEqual(result.data.mode, 'manual',
      `default should apply with required=${JSON.stringify(required)}`);
  }
});

await runTest('an explicit value overrides a schema default', () => {
  const schema = {
    properties: {
      mode: { type: 'string', default: 'manual' },
    },
    required: [],
  };
  const result = validateArguments({ mode: 'auto' }, schema);
  assert.ok(result.success);
  assert.strictEqual(result.data.mode, 'auto');
});

await runTest('createValidator produces reusable validator', () => {
  const schema = {
    properties: {
      name: { type: 'string' },
    },
    required: ['name'],
  };
  const validator = createValidator(schema);
  const ok = validator.safeParse({ name: 'foo' });
  assert.ok(ok.success);
  const bad = validator.safeParse({});
  assert.ok(!bad.success);
});

// ---------------------------------------------------------------------------
// cli-executor buildCliArgs tests
// ---------------------------------------------------------------------------

console.log('\ncli-executor.js (buildCliArgs)');

await runTest('positional args come before flags', () => {
  const cmdDef = {
    args: [{ name: 'id', type: 'string' }],
    flags: [
      { name: 'json', type: 'boolean' },
    ],
  };
  const result = buildCliArgs(['issue', 'show'], { id: 'abc123' }, cmdDef);
  // Expected: ['issue', 'show', 'abc123', '--json']
  const idIndex = result.indexOf('abc123');
  const jsonIndex = result.indexOf('--json');
  assert.ok(idIndex >= 0, 'should contain positional arg');
  assert.ok(jsonIndex >= 0, 'should contain --json');
  assert.ok(idIndex < jsonIndex, 'positional should come before flags');
});

await runTest('array values expand to repeated flags', () => {
  const cmdDef = {
    args: [],
    flags: [
      { name: 'title', type: 'string' },
      { name: 'label', type: 'array<string>' },
      { name: 'json', type: 'boolean' },
    ],
  };
  const result = buildCliArgs(['issue', 'create'], {
    title: 'Test',
    label: ['type:task', 'component:api'],
  }, cmdDef);
  // Should have --label type:task --label component:api
  const labelIndices = result.reduce((acc, v, i) => v === '--label' ? [...acc, i] : acc, []);
  assert.strictEqual(labelIndices.length, 2, 'should have two --label flags');
  assert.strictEqual(result[labelIndices[0] + 1], 'type:task');
  assert.strictEqual(result[labelIndices[1] + 1], 'component:api');
});

await runTest('boolean true emits bare flag, false omits', () => {
  const cmdDef = {
    args: [],
    flags: [
      { name: 'fix', type: 'boolean' },
      { name: 'json', type: 'boolean' },
    ],
  };
  const withTrue = buildCliArgs(['validate'], { fix: true }, cmdDef);
  assert.ok(withTrue.includes('--fix'), 'true should emit --fix');

  const withFalse = buildCliArgs(['validate'], { fix: false }, cmdDef);
  assert.ok(!withFalse.includes('--fix'), 'false should omit --fix');
});

await runTest('auto-injects --json when command supports it', () => {
  const cmdDef = {
    args: [],
    flags: [
      { name: 'title', type: 'string' },
      { name: 'json', type: 'boolean' },
    ],
  };
  const result = buildCliArgs(['issue', 'create'], { title: 'Test' }, cmdDef);
  assert.ok(result.includes('--json'), 'should auto-inject --json');
});

await runTest('does not inject --json when command lacks json flag', () => {
  const cmdDef = {
    args: [],
    flags: [
      { name: 'title', type: 'string' },
    ],
  };
  const result = buildCliArgs(['issue', 'create'], { title: 'Test' }, cmdDef);
  assert.ok(!result.includes('--json'), 'should not inject --json');
});

await runTest('does not duplicate --json when already provided', () => {
  const cmdDef = {
    args: [],
    flags: [
      { name: 'json', type: 'boolean' },
    ],
  };
  const result = buildCliArgs(['status'], { json: true }, cmdDef);
  const jsonCount = result.filter(a => a === '--json').length;
  assert.strictEqual(jsonCount, 1, 'should have exactly one --json');
});

await runTest('skips undefined and empty string values', () => {
  const cmdDef = {
    args: [{ name: 'id', type: 'string' }],
    flags: [
      { name: 'title', type: 'string' },
      { name: 'json', type: 'boolean' },
    ],
  };
  const result = buildCliArgs(['issue', 'show'], { id: 'abc', title: undefined }, cmdDef);
  assert.ok(!result.includes('--title'), 'should skip undefined flag');
  assert.ok(!result.includes('undefined'), 'should not have literal undefined');
});

await runTest('a positional argument defaulted to the empty string stays off the command line', () => {
  // `item search` declares an empty-string default for its query, which
  // validation materialises. An empty positional would shift every argument
  // after it, so it must be dropped here.
  const cmdDef = {
    args: [{ name: 'query', type: 'string' }],
    flags: [{ name: 'json', type: 'boolean' }],
  };
  const result = buildCliArgs(['item', 'search'], { query: '' }, cmdDef);
  assert.deepStrictEqual(result, ['item', 'search', '--json']);
});

await runTest('command path forms the start of args', () => {
  const cmdDef = { args: [], flags: [] };
  const result = buildCliArgs(['doc', 'assets', 'list'], {}, cmdDef);
  assert.deepStrictEqual(result.slice(0, 3), ['doc', 'assets', 'list']);
});

await runTest('multiple positional args appear in order', () => {
  const cmdDef = {
    args: [
      { name: 'id', type: 'string' },
      { name: 'path', type: 'string' },
    ],
    flags: [{ name: 'json', type: 'boolean' }],
  };
  const result = buildCliArgs(['doc', 'show'], { id: 'abc', path: 'readme.md' }, cmdDef);
  const idIndex = result.indexOf('abc');
  const pathIndex = result.indexOf('readme.md');
  assert.ok(idIndex < pathIndex, 'id should come before path');
  assert.ok(idIndex > 1, 'positional args should come after command path');
});

await runTest('array positional arg expands to multiple CLI args (e.g. dep add to_ids)', () => {
  const cmdDef = {
    args: [
      { name: 'from_id', type: 'string' },
      { name: 'to_ids', type: 'array' },
    ],
    flags: [{ name: 'json', type: 'boolean' }],
  };
  const result = buildCliArgs(
    ['dep', 'add'],
    { from_id: '350bff7f', to_ids: ['bfe0ba7b', '2248b17d'] },
    cmdDef
  );
  assert.deepStrictEqual(result, ['dep', 'add', '350bff7f', 'bfe0ba7b', '2248b17d', '--json']);
});

// ---------------------------------------------------------------------------
// cli-executor getTimeoutForCommand tests
// ---------------------------------------------------------------------------

console.log('\ncli-executor.js (getTimeoutForCommand)');

const LONG_TIMEOUT = 600000;
const DEFAULT_TIMEOUT = 30000;

await runTest('gate evaluate gets the long timeout', () => {
  assert.strictEqual(getTimeoutForCommand(['gate', 'evaluate']), LONG_TIMEOUT);
});

await runTest('gate evaluate-all gets the long timeout', () => {
  assert.strictEqual(getTimeoutForCommand(['gate', 'evaluate-all']), LONG_TIMEOUT);
});

await runTest('gate status stays on the default timeout (inspection only)', () => {
  assert.strictEqual(getTimeoutForCommand(['gate', 'status']), DEFAULT_TIMEOUT);
});

await runTest('gate status-all stays on the default timeout (inspection only)', () => {
  assert.strictEqual(getTimeoutForCommand(['gate', 'status-all']), DEFAULT_TIMEOUT);
});

await runTest('gate fail stays on the default timeout (no checker execution)', () => {
  assert.strictEqual(getTimeoutForCommand(['gate', 'fail']), DEFAULT_TIMEOUT);
});

await runTest('a non-gate command gets the default timeout', () => {
  assert.strictEqual(getTimeoutForCommand(['issue', 'create']), DEFAULT_TIMEOUT);
});

await runTest('adding an unrelated gate verb does not disable the branch for existing verbs', () => {
  // Regression guard for the historical bug where only the first `||`
  // operand was a real comparison: a verb not in the long-timeout set
  // (e.g. 'list') must not affect the verdict for verbs that are.
  assert.strictEqual(getTimeoutForCommand(['gate', 'list']), DEFAULT_TIMEOUT);
  assert.strictEqual(getTimeoutForCommand(['gate', 'evaluate']), LONG_TIMEOUT);
});

// ---------------------------------------------------------------------------
// concurrency limiter tests
// ---------------------------------------------------------------------------

console.log('\nconcurrency.js');

await runTest('runs functions within limit', async () => {
  const limiter = new ConcurrencyLimiter(2);
  const results = [];
  await Promise.all([
    limiter.run(async () => { results.push(1); }),
    limiter.run(async () => { results.push(2); }),
  ]);
  assert.deepStrictEqual(results.sort(), [1, 2]);
});

await runTest('enforces max concurrent limit', async () => {
  const limiter = new ConcurrencyLimiter(2);
  let maxSeen = 0;

  const task = () => limiter.run(async () => {
    maxSeen = Math.max(maxSeen, limiter.running);
    // Yield to let others start
    await new Promise(r => setImmediate(r));
  });

  await Promise.all([task(), task(), task(), task(), task()]);
  assert.ok(maxSeen <= 2, `max concurrent should be <= 2, was ${maxSeen}`);
});

await runTest('queues excess requests and drains them', async () => {
  const limiter = new ConcurrencyLimiter(1);
  const order = [];

  const slow = () => limiter.run(async () => {
    order.push('a-start');
    await new Promise(r => setTimeout(r, 50));
    order.push('a-end');
  });

  const fast = () => limiter.run(async () => {
    order.push('b');
  });

  await Promise.all([slow(), fast()]);
  // 'b' should start after 'a-end' because limit is 1
  assert.ok(order.indexOf('b') > order.indexOf('a-start'),
    'second task should start after first');
});

await runTest('getStats reports running and queued', async () => {
  const limiter = new ConcurrencyLimiter(1);
  let statsWhileRunning;

  const blocker = limiter.run(async () => {
    await new Promise(r => setTimeout(r, 100));
  });

  // Give the blocker time to start
  await new Promise(r => setTimeout(r, 10));

  // Queue a second task
  const queued = limiter.run(async () => {});

  // Check stats while blocker is running and queued task is waiting
  statsWhileRunning = limiter.getStats();
  assert.strictEqual(statsWhileRunning.running, 1);
  assert.strictEqual(statsWhileRunning.queued, 1);

  await Promise.all([blocker, queued]);

  const statsAfter = limiter.getStats();
  assert.strictEqual(statsAfter.running, 0);
  assert.strictEqual(statsAfter.queued, 0);
});

await runTest('propagates errors from tasks', async () => {
  const limiter = new ConcurrencyLimiter(2);
  await assert.rejects(
    () => limiter.run(async () => { throw new Error('boom'); }),
    { message: 'boom' }
  );
  // Limiter should still work after error
  const result = await limiter.run(async () => 42);
  assert.strictEqual(result, 42);
});

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
