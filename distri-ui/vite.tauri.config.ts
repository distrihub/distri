import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src')
    }
  },
  build: {
    outDir: 'dist-tauri',
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, 'index-tauri.html')
      }
    }
  },
  css: {
    postcss: './postcss.config.js'
  },
  define: {
    'process.env.TAURI_BUILD': 'true'
  },
  // Tauri expects the dev server to run on port 5174
  server: {
    port: 5174,
    strictPort: true
  },
  // prevent vite from obscuring rust errors
  clearScreen: false,
  // Tauri expects a fixed port, fail if that port is not available
  envPrefix: ['VITE_', 'TAURI_'],
})