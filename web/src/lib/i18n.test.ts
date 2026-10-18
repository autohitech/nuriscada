// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
import { LOCALES, detectLocale, eventLabel, i18n, m, setLocale, stateLabel } from "./i18n.svelte";
import { en } from "./locales/en";
import { ko } from "./locales/ko";

/** 객체의 모든 (키 경로, 값) 쌍(중첩 포함). 키에 점이 있어도 안전하도록 경로를 배열로 모은다. */
function entries(o: Record<string, unknown>, prefix: string[] = []): Array<[string[], unknown]> {
  return Object.entries(o).flatMap(([k, v]) =>
    v && typeof v === "object" ? entries(v as Record<string, unknown>, [...prefix, k]) : [[[...prefix, k], v] as [string[], unknown]],
  );
}

const paths = (o: Record<string, unknown>) => entries(o).map(([p]) => p.join("\u0000"));

describe("dictionaries", () => {
  it("English has exactly the same keys as Korean", () => {
    expect(paths(en as unknown as Record<string, unknown>).sort()).toEqual(
      paths(ko as unknown as Record<string, unknown>).sort(),
    );
  });

  it("parameterized messages take the same number of arguments", () => {
    for (const [key, value] of Object.entries(ko)) {
      if (typeof value === "function") {
        const other = (en as unknown as Record<string, unknown>)[key];
        expect(typeof other, key).toBe("function");
        expect((other as (...a: never[]) => string).length, key).toBe(value.length);
      }
    }
  });

  it("no message is empty and English contains no Hangul", () => {
    const hangul = /[가-힣]/;
    for (const [path, value] of entries(en as unknown as Record<string, unknown>)) {
      if (typeof value === "string") {
        expect(value.length, path.join(".")).toBeGreaterThan(0);
        expect(hangul.test(value), path.join(".")).toBe(false);
      }
    }
    expect(hangul.test(en.writeConfirm("A", "1"))).toBe(false);
    expect(hangul.test(en.trendStats(1, "0", "1", "1"))).toBe(false);
  });
});

describe("detectLocale", () => {
  it("prefers a saved choice", () => {
    expect(detectLocale(["ko-KR"], "en")).toBe("en");
    expect(detectLocale(["en-US"], "ko")).toBe("ko");
  });
  it("ignores an unsupported saved choice", () => {
    expect(detectLocale(["ko-KR"], "xx")).toBe("ko");
  });
  it("uses the first supported browser language", () => {
    expect(detectLocale(["fr-FR", "ko-KR", "en-US"])).toBe("ko");
    expect(detectLocale(["EN-gb"])).toBe("en");
  });
  it("falls back to English", () => {
    expect(detectLocale(["fr-FR", "ja-JP"])).toBe("en");
    expect(detectLocale([])).toBe("en");
  });
});

describe("setLocale", () => {
  it("switches every message and the label helpers", () => {
    setLocale("ko", false);
    expect(m.login).toBe(ko.login);
    expect(stateLabel("HIHI")).toBe("상상한");
    expect(eventLabel("RETURN")).toBe("복귀");
    setLocale("en", false);
    expect(i18n.locale).toBe("en");
    expect(m.login).toBe(en.login);
    expect(stateLabel("HIHI")).toBe("High-high");
    expect(eventLabel("RETURN")).toBe("Returned");
    expect(m.devicesSummary(10, 9)).toBe("9 of 10 devices connected");
    // 모르는 값은 그대로 보인다.
    expect(stateLabel("WHAT")).toBe("WHAT");
    setLocale("ko", false);
  });

  it("covers every supported locale", () => {
    expect(LOCALES).toEqual(["ko", "en"]);
  });
});
