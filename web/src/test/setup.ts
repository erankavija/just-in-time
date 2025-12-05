import '@testing-library/jest-dom';

// Cleanup is handled automatically by @testing-library/react in React 18+
// No need for explicit afterEach cleanup

// Set React act environment flag for React 19 compatibility
// @ts-expect-error - Global flag for React testing
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
