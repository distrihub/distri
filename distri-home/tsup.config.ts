import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/index.ts', 'src/globals.css'],
  format: ['cjs', 'esm'],
  dts: true,
  splitting: false,
  sourcemap: true,
  clean: true,
  external: ['react', 'react-dom', '@distri/core', '@distri/react'],
  esbuildOptions(options) {
    options.banner = {
      js: '"use client";',
    };
  },
});
