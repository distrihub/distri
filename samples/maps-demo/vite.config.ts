import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  // Base path - use VITE_BASE env var if set, otherwise default
  base: process.env.VITE_BASE || '/',
  server: {
    port: 3009,
    open: true
  },
  build: {
    outDir: 'dist',
    sourcemap: true
  },
  optimizeDeps: {
    include: ['@vis.gl/react-google-maps']
  }
})