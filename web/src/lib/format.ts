// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import type { AlarmEvent, AlarmState } from "./types";

export type Severity = "normal" | "warn" | "critical" | "info" | "bad";

/** 알람 상태의 심각도. 색에만 의존하지 않도록 화면은 이 값으로 아이콘과 문구를 함께 고른다. */
export function severityOfState(state: AlarmState | null): Severity {
  switch (state) {
    case "HIHI":
    case "LOLO":
      return "critical";
    case "HIGH":
    case "LOW":
      return "warn";
    case "MIDDLE":
      return "info";
    default:
      return "normal";
  }
}

/** 알람 기록 종류의 심각도. */
export function severityOfEvent(event: AlarmEvent): Severity {
  switch (event) {
    case "HIHI":
    case "LOLO":
      return "critical";
    case "HIGH":
    case "LOW":
      return "warn";
    case "DI_ON":
    case "DI_OFF":
      return "info";
    default:
      return "normal";
  }
}

const number = new Intl.NumberFormat("ko-KR", { maximumFractionDigits: 2 });

/** 값을 화면 문자열로. 숫자는 소수 둘째 자리까지, 불리언은 문구로 바꾼다. */
export function formatValue(value: number | boolean | null, on = "켜짐", off = "꺼짐", none = "—"): string {
  if (value === null) return none;
  if (typeof value === "boolean") return value ? on : off;
  if (Object.is(value, -0)) return number.format(0);
  return number.format(value);
}

// 24시간제 `HH:MM:SS`. 언어와 무관하게 같은 모양으로 보이도록 영국식 로캘을 쓴다.
const clock = new Intl.DateTimeFormat("en-GB", {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

/** UTC 밀리초를 현지 시각 `HH:MM:SS`로. 0이면(읽은 적 없음) 빈 문자열. */
export function formatTime(ms: number): string {
  if (!ms) return "";
  return clock.format(new Date(ms));
}

/** 마이크로초를 밀리초 문자열로(소수 한 자리). */
export function formatMicros(us: number | null): string {
  if (us === null) return "—";
  return `${(us / 1000).toFixed(1)} ms`;
}
