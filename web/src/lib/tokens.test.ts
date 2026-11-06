// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..", "..");

function definedTokens(): Set<string> {
  const css = readFileSync(join(root, "src/styles/tokens.css"), "utf-8");
  return new Set(
    [...css.matchAll(/--([a-z0-9-]+)\s*:/g)].map((m) => m[1] ?? "").filter((t) => t !== ""),
  );
}

function usedTokens(): Set<string> {
  const used = new Set<string>();
  const walk = (dir: string): void => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        walk(path);
        continue;
      }
      if (!/\.(svelte|ts|css)$/.test(name) || name.endsWith(".test.ts")) continue;
      const text = readFileSync(path, "utf-8");
      for (const m of text.matchAll(/var\(--([a-z0-9-]+)/g)) {
        const token = m[1];
        if (token !== undefined) used.add(token);
      }
    }
  };
  walk(join(root, "src"));
  return used;
}

describe("design tokens", () => {
  it("defines every used token", () => {
    const defined = definedTokens();
    const missing = [...usedTokens()].filter((t) => !defined.has(t));
    expect(missing).toEqual([]);
  });
});
