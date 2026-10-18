// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// 개발 서버는 /api(웹소켓 포함)를 로컬 서버로 넘긴다. 빌드 결과는 외부 자원 없이 동작한다.
export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:8080", ws: true },
    },
  },
  build: { target: "es2022", sourcemap: false },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
