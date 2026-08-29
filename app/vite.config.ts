/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import solid from 'vite-plugin-solid'

// Tauri sets TAURI_DEV_HOST when the dev server must be reachable from a device.
const host = process.env.TAURI_DEV_HOST

export default defineConfig(({ mode }) => ({
  // solid-refresh is hot-reload machinery; under the test runner it has no
  // module graph to attach to and fails to resolve itself.
  plugins: [solid({ hot: mode !== 'test' })],
  clearScreen: false,
  build: {
    // flag-icons declares 271 countries twice over. Inlining them would put
    // every flag into the stylesheet the webview parses at each launch, for
    // the handful a room actually shows; as files they are fetched on use.
    assetsInlineLimit: (file) => (file.endsWith('.svg') ? false : undefined),
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  // Tests run through this same config, so components are compiled by
  // vite-plugin-solid rather than a generic JSX runtime — the tree under test
  // is the tree that ships.
  test: {
    environment: 'happy-dom',
    include: ['src/**/*.test.{ts,tsx}'],
  },
}))
