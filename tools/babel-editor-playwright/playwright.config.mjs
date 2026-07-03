import { defineConfig, devices } from '@playwright/test';
import path from 'node:path';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');
const defaultBabelApp = 'C:/Users/User/Desktop/dev/babel/tools/babel-editor-rebuild/app';
const babelEditorApp = process.env.BABEL_EDITOR_APP ?? defaultBabelApp;
const port = Number(process.env.BABEL_EDITOR_PORT ?? 53213);
const baseURL = process.env.BABEL_EDITOR_URL ?? `http://127.0.0.1:${port}/`;

export default defineConfig({
  testDir: './tests',
  outputDir: path.join(repoRoot, 'target', 'babel-editor-playwright'),
  timeout: 60_000,
  expect: {
    timeout: 5_000
  },
  fullyParallel: false,
  workers: 1,
  reporter: [['list']],
  use: {
    baseURL,
    browserName: 'chromium',
    permissions: ['clipboard-read', 'clipboard-write'],
    viewport: { width: 1280, height: 720 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off'
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] }
    }
  ],
  webServer: process.env.BABEL_EDITOR_URL
    ? undefined
    : {
        command: `npm --prefix "${babelEditorApp}" run dev -- --port ${port}`,
        url: baseURL,
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
        stdout: 'pipe',
        stderr: 'pipe'
      }
});
