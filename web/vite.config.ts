import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import packageMetadata from './package.json'

export default defineConfig({
  plugins: [react()],
  define: {
    __JIT_PRODUCT_VERSION__: JSON.stringify(packageMetadata.version),
  },
  build: {
    rollupOptions: {
      output: {
        // Keep shared dependencies behind their existing dynamic import boundaries.
        onlyExplicitManualChunks: true,
        manualChunks(id) {
          if (/\/node_modules\/(react|react-dom|scheduler)\//.test(id)) return 'react'
          if (/\/node_modules\/katex\//.test(id)) return 'math'
          if (/\/node_modules\/(@reactflow|reactflow)\//.test(id)) return 'graph'
          // Bound the complete lazy registry without a language allowlist.
          const grammar = /\/node_modules\/refractor\/lang\/([^/]+)\.js$/.exec(id)
          if (grammar) return grammar[1][0] < 'n' ? 'syntax-a-m' : 'syntax-n-z'
        },
      },
    },
  },
})
