// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
import { validateProjectSpec } from "./validate";
import type { ProjectSpec } from "./types";

function spec(over: Partial<ProjectSpec> = {}): ProjectSpec {
  return {
    server: { listen: "127.0.0.1:8080", publish_ms: 50 },
    device: [{ name: "sim", driver: "modbus-tcp", address: "127.0.0.1:5020", timeout_ms: 500, scan_ms: 100 }],
    tag: [],
    ...over,
  };
}

const tag = (name: string, device = "sim") => ({
  writable: false,
  name,
  device,
  unit: 1,
  space: "holding" as const,
  address: 0,
  type: "analog" as const,
  scale: { plc_base: 0, plc_full: 100, base: 0, full: 100, profile: 0, delay: 0, range: 0 },
  alarm: null,
  unit_text: "",
  value: null,
  length: null,
  swap_bytes: false,
  interlock: [],
});

describe("validateProjectSpec", () => {
  it("accepts a clean project", () => {
    expect(validateProjectSpec(spec({ tag: [tag("A"), tag("B")] }))).toEqual([]);
  });
  it("finds empty, duplicate and dangling tags", () => {
    const issues = validateProjectSpec(
      spec({ tag: [tag("A"), tag("A"), tag(""), tag("B", "missing")] }),
    );
    expect(issues.map((i) => i.code)).toEqual(["dup-tag-name", "empty-tag-name", "unknown-device"]);
  });
  it("finds duplicate screens and widgets", () => {
    const issues = validateProjectSpec(
      spec({
        screen: [
          { name: "S", width: 800, height: 480, widget: [{ id: "w", kind: "value", label: "", tag: null, visible_tag: null, rotate_tag: null, move_tag: null, x: 0, y: 0, width: 160, height: 120, min: 0, max: 100 }] },
          { name: "S", width: 800, height: 480, widget: [{ id: "w", kind: "value", label: "", tag: null, visible_tag: null, rotate_tag: null, move_tag: null, x: 0, y: 0, width: 160, height: 120, min: 0, max: 100 }, { id: "w", kind: "value", label: "", tag: null, visible_tag: null, rotate_tag: null, move_tag: null, x: 0, y: 0, width: 160, height: 120, min: 0, max: 100 }] },
        ],
      }),
    );
    expect(issues.map((i) => i.code)).toEqual(["dup-screen-name", "dup-widget-id"]);
  });
});
