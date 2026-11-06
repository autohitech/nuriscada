// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { defineConfig, devices } from "@playwright/test";

// `scripts/visual.sh`이 시뮬레이터·서버·웹 빌드를 띄운 뒤 실행한다.
// baseURL은 환경 변수 PLAYWRIGHT_BASE_URL로 덮을 수 있다.
export default defineConfig({
  testDir: "./tests",
  snapshotPathTemplate: "{testDir}/__screenshots__/{projectName}-{platform}/{arg}{ext}",
  use: {
    baseURL: process.env.PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:18080",
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
});
