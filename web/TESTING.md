# Web UI Testing Guide

## Test Suite Overview

The web UI has comprehensive tests for the state model refactoring using **Vitest** and **React Testing Library**.

## Setup

```bash
npm install
```

This installs:
- `vitest` - Fast unit test framework for Vite
- `@testing-library/react` - React component testing utilities
- `@testing-library/jest-dom` - DOM assertion matchers
- `jsdom` - DOM implementation for Node.js

## Running Tests

```bash
# Run all tests
npm test

# Run tests in watch mode
npm run test:watch
```

## Test Files

### Type Tests (`src/types/__tests__/models.test.ts`)
✅ Validates new State type includes `backlog` and `gated`  
✅ Confirms old `open` state is removed  
✅ Verifies Issue type structure with new states  
✅ Tests gated state with gates_status

### Graph Component Tests (`src/components/Graph/__tests__/stateColors.test.ts`)
✅ Validates stateColors mapping has all 6 states  
✅ Confirms colors use CSS variables  
✅ Checks backlog and gated colors are defined  

### Issue Component Tests (`src/components/Issue/__tests__/stateEmojis.test.ts`)
✅ Validates stateEmoji mapping has all 6 states  
✅ Confirms unique emojis for each state  
✅ Verifies meaningful emojis (⏸️ for backlog, 🟠 for gated)  

### CSS Variable Tests (`src/__tests__/css-variables.test.ts`)
✅ Validates --state-backlog variable exists  
✅ Validates --state-gated variable exists  
✅ Confirms --state-open does NOT exist  
✅ Checks all 6 states in dark and light themes  

## Test Coverage

The test suite specifically validates the state model refactoring:

| Component | Test Coverage |
|-----------|--------------|
| TypeScript Types | ✅ All 6 states type-checked |
| Graph Colors | ✅ CSS variable mappings |
| Issue Emojis | ✅ Visual indicators |
| CSS Variables | ✅ Dark & light themes |

## Key Validations

1. **State Enum**: Backlog, Ready, InProgress, Gated, Done, Archived
2. **No "Open"**: Old state completely removed
3. **CSS Consistency**: Variables match type definitions
4. **Visual Indicators**: Unique emoji/color per state

## Example Test Output

```
✓ src/types/__tests__/models.test.ts (4)
   ✓ State type (3)
     ✓ should include all new state values
     ✓ should have exactly 6 states  
   ✓ Issue type structure (2)
     ✓ should accept gated state

✓ src/components/Graph/__tests__/stateColors.test.ts (1)
   ✓ GraphView state colors (1)
     ✓ should have colors defined for all 6 states

✓ src/components/Issue/__tests__/stateEmojis.test.ts (2)
   ✓ IssueDetail state emojis (2)
     ✓ should have emojis for all 6 states
     ✓ should have unique emojis for each state

✓ src/__tests__/css-variables.test.ts (4)
   ✓ CSS state variables (4)
     ✓ should define --state-backlog variable
     ✓ should define --state-gated variable
     ✓ should not define --state-open variable
     ✓ should define all 6 state colors

Test Files  4 passed (4)
     Tests  11 passed (11)
```

## Configuration

Tests are configured in `vitest.config.ts`:
- Uses jsdom environment for DOM testing
- Loads `@testing-library/jest-dom` matchers
- Integrates with Vite build configuration

## CI Integration

Add to CI/CD pipeline:

```yaml
- name: Test Web UI
  run: |
    cd web
    npm install
    npm test
```

## Troubleshooting

### Tests not finding modules
Ensure all dependencies are installed:
```bash
npm install
```

### TypeScript errors
The tests use `@ts-expect-error` to verify that invalid states (like `'open'`) are correctly rejected by TypeScript.

### CSS file not found
Tests read `src/index.css` - ensure it exists and contains state variable definitions.

## Adding New Tests

Follow the pattern:

```typescript
import { describe, it, expect } from 'vitest';
import type { State } from '../types/models';

describe('New feature', () => {
  it('should validate state behavior', () => {
    const state: State = 'gated';
    expect(state).toBe('gated');
  });
});
```

## See Also

- [Vitest Documentation](https://vitest.dev/)
- [React Testing Library](https://testing-library.com/react)
- [State Model Refactoring](../docs/state-model-refactoring.md)
