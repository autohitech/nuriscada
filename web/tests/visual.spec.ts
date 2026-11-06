// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

// 시각 회귀의 출발점. 로그인 화면은 고정 스냅샷으로 견주고,
// 런타임 화면은 값이 계속 바뀌므로 구조 확인 + 촬영본 보관만 한다.
import { expect, test } from "@playwright/test";

const ADMIN = { username: "visual", password: "visual-test-password-123" };

test("login view matches the committed snapshot", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();
  await expect(page).toHaveScreenshot("login.png", { maxDiffPixelRatio: 0.02 });
});

test("runtime view loads live data", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Account", { exact: true }).fill(ADMIN.username);
  await page.getByLabel("Password", { exact: true }).fill(ADMIN.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("tablist").first()).toBeVisible({ timeout: 15000 });
  // 시뮬레이터를 고정값으로 띄우므로 화면이 안정적이다. 전체를 견준다.
  await expect(page.locator("main")).toHaveScreenshot("runtime.png", {
    maxDiffPixelRatio: 0.02,
  });
  await page.screenshot({ path: "test-results/runtime-full.png", fullPage: true });
});
