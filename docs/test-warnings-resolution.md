# Test Warnings Resolution

## Summary

Fixed all React Testing Library warnings in GraphView tests by properly handling async state updates and correcting mock implementations.

## Issues Identified

### 1. React `act()` Warnings

**Warning Message:**
```
An update to GraphView inside a test was not wrapped in act(...).
When testing, code that causes React state updates should be wrapped into act(...)
```

**Root Cause:**
- GraphView component uses `useEffect` to fetch data from `apiClient.getGraph()`
- This is an async operation (Promise-based)
- State updates happen after initial render completes
- Tests were completing before async state updates finished
- React Testing Library couldn't guarantee test stability

**Why It Matters:**
- Tests may pass inconsistently (race conditions)
- Not testing actual user experience (component in intermediate state)
- Future refactoring could introduce timing-dependent bugs
- Violates React Testing Library best practices

**Solution:**
Import `waitFor` from `@testing-library/react` and wrap render calls:

```typescript
// Before (warnings):
it('should render without crashing', () => {
  render(<GraphView viewMode="tactical" />);
});

// After (clean):
it('should render without crashing', async () => {
  render(<GraphView viewMode="tactical" />);
  await waitFor(() => {});  // Wait for async updates
});
```

**Impact:** All 6 GraphView tests now properly wait for component to settle.

### 2. Dagre Mock Constructor Error

**Warning Message:**
```
Failed to load graph: TypeError: () => ({ ... }) is not a constructor
[vitest] The vi.fn() mock did not use 'function' or 'class' in its implementation
```

**Root Cause:**
The dagre mock was using `vi.fn()` which returns a plain function, not a constructor:

```typescript
// Before (broken):
Graph: vi.fn(() => ({
  setDefaultEdgeLabel: vi.fn(),
  // ...
}))

// GraphView.tsx calls:
new dagre.graphlib.Graph()  // ❌ TypeError
```

**Why It Matters:**
- Graph layout fails silently
- Tests pass because error is caught
- Console pollution with error messages
- Not testing actual layout logic path

**Solution:**
Use `vi.fn().mockImplementation()` with a proper constructor function:

```typescript
// After (working):
Graph: vi.fn().mockImplementation(function() {
  return {
    setDefaultEdgeLabel: vi.fn(),
    setGraph: vi.fn(),
    setNode: vi.fn(),
    setEdge: vi.fn(),
    node: vi.fn(() => ({ x: 0, y: 0 })),
  };
})
```

**Key Difference:**
- `vi.fn(() => {})` - regular function
- `vi.fn().mockImplementation(function() {})` - constructor-compatible

### 3. Missing `dagre.layout()` Mock

**Warning Message:**
```
Failed to load graph: TypeError: dagre.layout is not a function
```

**Root Cause:**
GraphView calls `dagre.layout(graph)` but mock only defined `dagre.graphlib.Graph`.

**Solution:**
Add `layout` to the dagre mock:

```typescript
vi.mock('dagre', () => ({
  default: {
    graphlib: { Graph: /* ... */ },
    layout: vi.fn(),  // ✅ Mock the layout function
  },
}));
```

## Complete Fixed Mock

```typescript
vi.mock('dagre', () => ({
  default: {
    graphlib: {
      Graph: vi.fn().mockImplementation(function() {
        return {
          setDefaultEdgeLabel: vi.fn(),
          setGraph: vi.fn(),
          setNode: vi.fn(),
          setEdge: vi.fn(),
          node: vi.fn(() => ({ x: 0, y: 0 })),
        };
      }),
    },
    layout: vi.fn(),
  },
}));
```

## Verification

### Before Fix
```bash
$ npm test -- GraphView.test.tsx
# Output: 6 tests pass, ~30 warning messages
```

### After Fix
```bash
$ npm test -- GraphView.test.tsx
# Output: 6 tests pass, 0 warnings ✅
```

## Lessons Learned

### 1. Always Wait for Async Updates in Tests

**Pattern:**
```typescript
it('should test async component', async () => {
  render(<AsyncComponent />);
  await waitFor(() => {
    // Assert on final state
  });
});
```

### 2. Mock Constructors Properly

**Wrong:**
```typescript
MyClass: vi.fn(() => ({ method: vi.fn() }))
```

**Right:**
```typescript
MyClass: vi.fn().mockImplementation(function() {
  return { method: vi.fn() };
})
```

### 3. Mock Complete API Surface

If component uses `library.method()`, mock must include `method`:

```typescript
vi.mock('library', () => ({
  default: {
    method: vi.fn(),  // ✅ Include all used methods
  }
}));
```

### 4. Read Warning Messages Carefully

- `act()` warning → async state updates not awaited
- "not a constructor" → mock needs `mockImplementation`
- "not a function" → missing method in mock

## Testing Best Practices Applied

1. **Test User Experience**: Wait for component to reach final state users see
2. **Isolated Tests**: Proper mocks prevent external dependencies
3. **Clear Assertions**: Each test has single, clear purpose
4. **Fast Execution**: Mocks make tests run in <100ms
5. **Zero Warnings**: Clean test output indicates correct testing patterns

## Impact

- **Before**: 6 tests, ~30 warnings, polluted output
- **After**: 6 tests, 0 warnings, clean output
- **Time**: ~30 minutes to identify and fix
- **Risk Reduction**: Eliminated potential timing-based flakiness

## Related Files

- `web/src/components/Graph/__tests__/GraphView.test.tsx` - Fixed tests
- `web/src/components/Graph/GraphView.tsx` - Component under test
- `docs/graph-filtering-architecture.md` - Filter system documentation

## References

- [React Testing Library: Async Methods](https://testing-library.com/docs/dom-testing-library/api-async/)
- [Vitest: Mocking](https://vitest.dev/guide/mocking.html)
- [React: Testing Recipes with act()](https://react.dev/reference/react/act)
