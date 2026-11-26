import { defineConfig, loadEnv } from 'vite';
import react from '@vitejs/plugin-react';
import { distriToken } from './server/distri-token';

// DISTRI_API_KEY has no VITE_ prefix, so it stays in this Node process. The
// browser asks /api/distri/token for a short-lived token instead.
export default defineConfig(({ mode }) => {
  const env = { ...process.env, ...loadEnv(mode, process.cwd(), '') };
  return {
    plugins: [
      react(),
      distriToken({
        apiKey: env.DISTRI_API_KEY ?? '',
        baseUrl: env.DISTRI_URL ?? 'https://api.distri.dev/v1',
        // Needed when the key is not tied to one workspace.
        workspaceId: env.DISTRI_WORKSPACE_ID || undefined,
      }),
    ],
    server: { port: Number(env.PORT ?? 5302), strictPort: true },
  };
});
