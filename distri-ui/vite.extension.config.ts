import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { crx } from '@crxjs/vite-plugin'
import manifest from './manifest.json'
import path from 'path'

export default defineConfig({
  plugins: [
    react(),
    crx({ manifest })
  ],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src')
    }
  },
  build: {
    outDir: 'dist-extension',
    rollupOptions: {
      input: {
        popup: path.resolve(__dirname, 'index-extension.html')
      }
    }
  },
  css: {
    postcss: './postcss.config.js'
  },
  define: {
    'process.env.EXTENSION_BUILD': 'true'
  }
})