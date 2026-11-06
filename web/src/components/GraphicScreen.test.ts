// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
import { render } from "svelte/server";
import GraphicScreen from "./GraphicScreen.svelte";
import { Store } from "../lib/store.svelte";
import type { TagSnapshot } from "../lib/types";

function tag(name: string, kind: "analog" | "digital", value: number | boolean): TagSnapshot {
  return {
    name,
    device: "sim",
    kind,
    value,
    quality: "good",
    changed_ms: 1,
    unit_text: "%",
    alarm_state: "NORMAL",
  };
}

describe("GraphicScreen", () => {
  it("renders widgets from live tags with bound attributes", () => {
    const store = new Store();
    store.apply({
      type: "snapshot",
      data: {
        seq: 1,
        tags: [tag("LEVEL", "analog", 50), tag("OTHER", "analog", 50), tag("PUMP", "digital", true)],
        active: [],
        recent: [],
      },
    });
    const { body } = render(GraphicScreen, { props: { store, canEditGraphic: false } });
    expect(body).toContain("graphic-svg");
    // 게이지 채움: 너비 170 → 막대 142의 절반.
    expect(body).toContain('width="71"');
    // 운전 중인 디지털 위젯은 켜짐 색을 쓴다.
    expect(body).toContain("var(--ok");
  });
});
