// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
import { Store, RECENT_LIMIT, connect } from "./store.svelte";
import { formatTime, formatValue, severityOfEvent, severityOfState } from "./format";
import { visibleRange } from "./window";
import type { AlarmRecord, StreamMessage, TagSnapshot } from "./types";

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

const snapshot = (tags: TagSnapshot[], recent: AlarmRecord[] = []): StreamMessage => ({
  type: "snapshot",
  data: { tags, active: tags.filter((t) => t.alarm_state && t.alarm_state !== "NORMAL"), recent },
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

  it("updates one tag in place and tracks the active set", () => {
    const s = new Store();
    s.apply(snapshot([tag("A"), tag("B")]));
    s.apply({ type: "tag", data: tag("A", { value: 95, quality: "good", alarm_state: "HIHI" }) });
    expect(s.tags[0]?.value).toBe(95);
    expect(s.tags[1]?.value).toBeNull();
    expect(s.activeNames).toEqual(["A"]);
    // 같은 알람 상태로 다시 와도 중복되지 않는다.
    s.apply({ type: "tag", data: tag("A", { value: 96, quality: "good", alarm_state: "HIHI" }) });
    expect(s.activeNames).toEqual(["A"]);
    s.apply({ type: "tag", data: tag("A", { value: 10, quality: "good", alarm_state: "NORMAL" }) });
    expect(s.activeNames).toEqual([]);
  });

  it("ignores updates for unknown tags", () => {
    const s = new Store();
    s.apply(snapshot([tag("A")]));
    s.apply({ type: "tag", data: tag("ZZZ", { value: 1 }) });
    expect(s.tags).toHaveLength(1);
  });

  it("keeps a bounded alarm log", () => {
    const s = new Store();
    s.apply(snapshot([tag("A")]));
    for (let i = 1; i <= RECENT_LIMIT + 50; i++) s.apply({ type: "alarm", data: alarm(i) });
    expect(s.recent).toHaveLength(RECENT_LIMIT);
    expect(s.recent[0]?.seq).toBe(51);
    expect(s.latestAlarm?.seq).toBe(RECENT_LIMIT + 50);
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
    expect(formatValue(true)).toBe("켜짐");
    expect(formatValue(false)).toBe("꺼짐");
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
