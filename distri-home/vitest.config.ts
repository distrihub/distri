import { defineConfig } from 'vitest/config';
import { resolve } from 'path';

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['src/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      include: ['src/**/*.ts'],
      exclude: ['src/**/*.test.ts', 'src/**/index.ts'],
    },
  },
  resolve: {
    alias: {
      '@distri/core': resolve(__dirname, './src/__mocks__/distri-core.ts'),
      '@distri/react': resolve(__dirname, './src/__mocks__/distri-react.ts'),
      '@': resolve(__dirname, './src'),
    },
  },
});
