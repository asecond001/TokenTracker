import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    rollupOptions: {
      output: {
        // Keep the chart engine in its own long-lived chunk so the tray and
        // floating windows (which never draw ECharts) skip it entirely.
        manualChunks(id) {
          if (id.includes('node_modules/zrender')) {
            return 'zrender';
          }
          if (id.includes('node_modules/echarts')) {
            return 'echarts';
          }
          return undefined;
        },
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/__tests__/setup.ts'],
  },
});
