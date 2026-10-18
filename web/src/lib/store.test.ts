// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
import { Store, RECENT_LIMIT, connect } from "./store.svelte";
import { formatTime, formatValue, severityOfEvent, severityOfState } from "./format";
import { visibleRange } from "./window";
import type { AlarmRecord, StreamMessage, TagItem, TagSnapshot } from "./types";

const tag = (name: string, patch: Partial<TagSnapshot> = {}): TagSnapshot => ({
  name,
  device: "sim",
  kind: "analog",
  value: null,
  quality: "bad",
  changed_ms: 0,
  unit_text: "%",
  alarm_state: "NORMAL",
  ...patch,
});

const alarm = (seq: number, event: AlarmRecord["event"] = "HIHI"): AlarmRecord => ({
  seq,
  time_ms: 1000 + seq,
  tag: "A",
  event,
  value: 1,
});

const snapshot = (tags: TagSnapshot[], recent: AlarmRecord[] = [], seq = 0): StreamMessage => ({
  type: "snapshot",
  data: { seq, tags, active: tags.filter((t) => t.alarm_state && t.alarm_state !== "NORMAL"), recent },
});

const batch = (seq: number, items: TagItem[]): StreamMessage => ({
  type: "tags",
  data: { seq, time_ms: 5000 + seq, items },
});

describe("Store", () => {
  it("applies a snapshot", () => {
    const s = new Store();
    s.apply(snapshot([tag("A"), tag("B", { alarm_state: "HIHI" })], [alarm(1)]));
    expect(s.names).toEqual(["A", "B"]);
    expect(s.activeNames).toEqual(["B"]);
    expect(s.recent).toHaveLength(1);
    expect(s.indexOf("B")).toBe(1);
  });

  it("applies batched items by index and tracks the active set", () => {
    const s = new Store();
    s.apply(snapshot([tag("A"), tag("B")]));
    s.apply(batch(1, [[0, 95, 1, "HIHI"]]));
    expect(s.tags[0]).toMatchObject({ name: "A", value: 95, quality: "good", alarm_state: "HIHI", changed_ms: 5001 });
    expect(s.tags[1]?.value).toBeNull();
    expect(s.activeNames).toEqual(["A"]);
    // 같은 알람 상태로 다시 와도 중복되지 않는다.
    s.apply(batch(2, [[0, 96, 1, "HIHI"]]));
    expect(s.activeNames).toEqual(["A"]);
    s.apply(batch(3, [[0, 10, 1, "NORMAL"]]));
    expect(s.activeNames).toEqual([]);
  });

  it("keeps the last value and alarm state when a read fails", () => {
    const s = new Store();
    s.apply(snapshot([tag("A", { value: 40, quality: "good", alarm_state: "HIGH" })]));
    s.apply(batch(1, [[0, null, 0, null]]));
    expect(s.tags[0]).toMatchObject({ value: 40, quality: "bad", alarm_state: "HIGH" });
  });

  it("ignores items for unknown positions", () => {
    const s = new Store();
    s.apply(snapshot([tag("A")]));
    s.apply(batch(1, [[7, 1, 1, null]]));
    expect(s.tags).toHaveLength(1);
  });

  it("keeps a bounded alarm log and skips records the snapshot already had", () => {
    const s = new Store();
    s.apply(snapshot([tag("A")], [alarm(1), alarm(2)]));
    s.apply({ type: "alarms", data: [alarm(2), alarm(3)] });
    expect(s.recent.map((r) => r.seq)).toEqual([1, 2, 3]);
    expect(s.latestAlarm?.seq).toBe(3);
    const many = Array.from({ length: RECENT_LIMIT + 50 }, (_, i) => alarm(10 + i));
    s.apply({ type: "alarms", data: many });
    expect(s.recent).toHaveLength(RECENT_LIMIT);
    expect(s.recent[s.recent.length - 1]?.seq).toBe(10 + RECENT_LIMIT + 49);
  });

  it("handles a full batch for 10,000 tags quickly", () => {
    const s = new Store();
    const tags = Array.from({ length: 10_000 }, (_, i) => tag(`T${i}`));
    s.apply(snapshot(tags));
    const items: TagItem[] = Array.from({ length: 1000 }, (_, k) => [k * 10, k, 1, null]);
    const t0 = performance.now();
    for (let n = 0; n < 20; n++) s.apply(batch(n + 1, items));
    const perBatch = (performance.now() - t0) / 20;
    // 느슨한 안전망이다(정확한 측정은 설계 0004의 방법을 따른다).
    expect(perBatch).toBeLessThan(100);
  });

  it("replaces state on a new snapshot (reconnect)", () => {
    const s = new Store();
    s.apply(snapshot([tag("A", { alarm_state: "HIHI" })], [alarm(1)]));
    s.apply(snapshot([tag("B")]));
    expect(s.names).toEqual(["B"]);
    expect(s.activeNames).toEqual([]);
    expect(s.recent).toEqual([]);
  });
});

describe("connect", () => {
  class FakeSocket {
    onopen: (() => void) | null = null;
    onmessage: ((ev: { data: string }) => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    closed = false;
    close() {
      this.closed = true;
      this.onclose?.();
    }
  }

  it("marks connected, applies messages, and reconnects after a drop", async () => {
    const sockets: FakeSocket[] = [];
    const store = new Store();
    const stop = connect({
      url: "ws://x/api/v1/stream",
      store,
      socket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s as unknown as WebSocket;
      },
      backoff: { initial: 1, max: 2 },
    });
    expect(sockets).toHaveLength(1);
    sockets[0]?.onopen?.();
    expect(store.connected).toBe(true);
    sockets[0]?.onmessage?.({ data: JSON.stringify(snapshot([tag("A")])) });
    expect(store.names).toEqual(["A"]);
    sockets[0]?.onmessage?.({ data: "not json" }); // 무시된다
    expect(store.names).toEqual(["A"]);

    sockets[0]?.onclose?.();
    expect(store.connected).toBe(false);
    await new Promise((r) => setTimeout(r, 20));
    expect(sockets).toHaveLength(2);

    stop();
    expect(sockets[1]?.closed).toBe(true);
    await new Promise((r) => setTimeout(r, 20));
    expect(sockets).toHaveLength(2); // 멈춘 뒤에는 다시 연결하지 않는다
  });
});

describe("format", () => {
  it("formats values", () => {
    expect(formatValue(null)).toBe("—");
    expect(formatValue(true)).toBe("ON");
    expect(formatValue(false)).toBe("OFF");
    expect(formatValue(12.345)).toBe("12.35");
    expect(formatValue(-0)).toBe("0");
    expect(formatValue(1000)).toBe("1,000");
  });

  it("formats clock time as 24h HH:MM:SS", () => {
    expect(formatTime(0)).toBe("");
    expect(formatTime(Date.now())).toMatch(/^\d{2}:\d{2}:\d{2}$/);
  });

  it("maps severities", () => {
    expect(severityOfState("HIHI")).toBe("critical");
    expect(severityOfState("LOW")).toBe("warn");
    expect(severityOfState("MIDDLE")).toBe("info");
    expect(severityOfState("NORMAL")).toBe("normal");
    expect(severityOfState(null)).toBe("normal");
    expect(severityOfEvent("LOLO")).toBe("critical");
    expect(severityOfEvent("RETURN")).toBe("normal");
    expect(severityOfEvent("DI_ON")).toBe("info");
  });
});

describe("visibleRange", () => {
  it("returns an overscanned window", () => {
    expect(visibleRange(0, 360, 36, 10_000, 8)).toEqual({ start: 0, end: 18 });
    expect(visibleRange(3600, 360, 36, 10_000, 8)).toEqual({ start: 92, end: 118 });
  });
  it("clamps to the list", () => {
    expect(visibleRange(0, 360, 36, 5, 8)).toEqual({ start: 0, end: 5 });
    expect(visibleRange(10_000_000, 360, 36, 100, 8)).toEqual({ start: 100, end: 100 });
    expect(visibleRange(0, 360, 36, 0)).toEqual({ start: 0, end: 0 });
  });
});
