import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import packageMetadata from './package.json'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  define: {
    __JIT_PRODUCT_VERSION__: JSON.stringify(packageMetadata.version),
  },
})
