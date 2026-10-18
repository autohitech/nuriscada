// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

// node_modules의 모든 패키지 라이선스가 허용 목록에 있는지 검사한다.
// 규칙: docs/requirements/dependency-license-policy.md (A 등급 허용, 그 밖은 검토).
// 사용: node scripts/check-licenses.mjs  (종료 코드 1이면 위반)

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ALLOWED = new Set([
  "MIT",
  "Apache-2.0",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
  "CC0-1.0",
  "Unlicense",
  "BlueOak-1.0.0",
  "Zlib",
  "MPL-2.0",
]);

/** SPDX 식 하나가 허용되는지. `OR`은 하나라도, `AND`는 모두 허용되어야 한다. */
function allowed(expr) {
  const flat = expr.replace(/[()]/g, " ").trim();
  if (/\sOR\s/.test(flat)) return flat.split(/\sOR\s/).some((p) => allowed(p));
  if (/\sAND\s/.test(flat)) return flat.split(/\sAND\s/).every((p) => allowed(p));
  return ALLOWED.has(flat.trim());
}

function* packages(dir) {
  for (const name of readdirSync(dir)) {
    if (name.startsWith(".")) continue;
    const full = join(dir, name);
    if (!statSync(full).isDirectory()) continue;
    if (name.startsWith("@")) {
      yield* packages(full);
      continue;
    }
    const manifest = join(full, "package.json");
    try {
      yield { dir: full, pkg: JSON.parse(readFileSync(manifest, "utf8")) };
    } catch {
      continue;
    }
    const nested = join(full, "node_modules");
    try {
      if (statSync(nested).isDirectory()) yield* packages(nested);
    } catch {
      // 중첩 패키지가 없다.
    }
  }
}

const bad = [];
let count = 0;
for (const { dir, pkg } of packages("node_modules")) {
  count++;
  const raw = pkg.license ?? (Array.isArray(pkg.licenses) ? pkg.licenses.map((l) => l.type).join(" OR ") : undefined);
  const license = typeof raw === "object" && raw !== null ? raw.type : raw;
  if (!license || !allowed(String(license))) bad.push(`${pkg.name}@${pkg.version}: ${license ?? "(없음)"} (${dir})`);
}
if (bad.length) {
  console.error(`허용되지 않은 라이선스 ${bad.length}건:\n` + bad.join("\n"));
  process.exit(1);
}
console.log(`라이선스 검사 통과 (${count}개 패키지)`);
