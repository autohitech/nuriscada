// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";

import { csvCell, formatImportCsv, isVisible, shouldBeep } from "./format";
import type { ImportReport } from "./types";

const report = (over: Partial<ImportReport> = {}): ImportReport => ({
  imported_devices: 1,
  imported_tags: 2,
  renamed_tags: [],
  modified_items: [],
  ignored_items: [],
  unreadable_items: [],
  ...over,
});

describe("csvCell", () => {
  it("quotes cells with commas, quotes, or newlines", () => {
    expect(csvCell("plain")).toBe("plain");
    expect(csvCell("a,b")).toBe('"a,b"');
    expect(csvCell('say "hi"')).toBe('"say ""hi"""');
    expect(csvCell("line1\nline2")).toBe('"line1\nline2"');
  });
});

describe("formatImportCsv", () => {
  it("writes a header row even for an empty report", () => {
    expect(formatImportCsv(report())).toBe("section,line,item,detail\r\n");
  });

  it("lists renamed, modified, ignored, and unreadable rows", () => {
    const csv = formatImportCsv(
      report({
        renamed_tags: [{ line: 3, original: "A", resolved: "A1" }],
        modified_items: [{ line: 4, item: "dev", detail: "고침" }],
        ignored_items: [{ line: 5, item: "ST", detail: '쉼표, 포함 "인용"' }],
        unreadable_items: [{ line: 6, item: "BAD", detail: "필드 부족" }],
      }),
    );
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe("section,line,item,detail");
    expect(lines).toContain("renamed,3,A,A -> A1");
    expect(lines).toContain("modified,4,dev,고침");
    expect(lines).toContain('ignored,5,ST,"쉼표, 포함 ""인용"""');
    expect(lines).toContain("unreadable,6,BAD,필드 부족");
    expect(csv.endsWith("\r\n")).toBe(true);
  });
});

describe("isVisible", () => {
  it("hides on bad quality or missing value (fail-closed)", () => {
    expect(isVisible(true, "bad")).toBe(false);
    expect(isVisible(1, "bad")).toBe(false);
    expect(isVisible(null, "good")).toBe(false);
  });
  it("shows only truthy values", () => {
    expect(isVisible(true, "good")).toBe(true);
    expect(isVisible(false, "good")).toBe(false);
    expect(isVisible(1, "good")).toBe(true);
    expect(isVisible(0, "good")).toBe(false);
    expect(isVisible("on", "good")).toBe(true);
    expect(isVisible("", "good")).toBe(false);
  });
});

describe("shouldBeep", () => {
  it("follows the configured rule", () => {
    expect(shouldBeep("off", "critical")).toBe(false);
    expect(shouldBeep("critical", "critical")).toBe(true);
    expect(shouldBeep("critical", "warn")).toBe(false);
    expect(shouldBeep("warn", "warn")).toBe(true);
    expect(shouldBeep("warn", "info")).toBe(false);
    expect(shouldBeep("all", "info")).toBe(true);
    expect(shouldBeep("all", "bad")).toBe(true);
    expect(shouldBeep("all", "normal")).toBe(false);
  });
});
