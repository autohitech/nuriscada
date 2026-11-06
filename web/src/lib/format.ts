// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { localeTag } from "./i18n.svelte";
import type { AlarmEvent, AlarmState, ImportReport } from "./types";

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

const numberFormats = new Map<string, Intl.NumberFormat>();

function numberFormat(): Intl.NumberFormat {
  const tag = localeTag();
  let f = numberFormats.get(tag);
  if (!f) {
    f = new Intl.NumberFormat(tag, { maximumFractionDigits: 2 });
    numberFormats.set(tag, f);
  }
  return f;
}

/** 값을 화면 문자열로. 숫자는 소수 둘째 자리까지, 불리언은 문구로, 문자열은 그대로 둔다. */
export function formatValue(value: number | boolean | string | null, on = "ON", off = "OFF", none = "—"): string {
  if (value === null) return none;
  if (typeof value === "boolean") return value ? on : off;
  if (typeof value === "string") return value;
  if (Object.is(value, -0)) return numberFormat().format(0);
  return numberFormat().format(value);
}
/** 가시성 조건: 값이 없거나 통신 불량이면 숨긴다(닫힘 실패). 0·거짓·빈 문자열도 숨긴다. */
export function isVisible(value: number | boolean | string | null, quality: "good" | "bad"): boolean {
  if (quality !== "good" || value === null) return false;
  if (typeof value === "boolean") return value;
  if (typeof value === "string") return value.length > 0;
  return value !== 0;
}
export type AlarmSoundRule = "off" | "critical" | "warn" | "all";

/** 소리 규칙에 따라 울릴지 정한다(순수 함수라 시험할 수 있다). */
export function shouldBeep(rule: AlarmSoundRule, severity: Severity): boolean {
  switch (rule) {
    case "off":
      return false;
    case "critical":
      return severity === "critical";
    case "warn":
      return severity === "critical" || severity === "warn";
    case "all":
      return severity !== "normal";
  }
}

/** RFC 4180 인용 규칙으로 한 셀을 감싼다. */
export function csvCell(text: string): string {
  if (/[",\r\n]/.test(text)) return `"${text.replace(/"/g, '""')}"`;
  return text;
}

/**
 * 가져오기 보고서를 CSV 텍스트로 만든다(RFC 4180, UTF-8).
 * 머리글: `section,line,item,detail`. 이름 변경은 `원래이름 -> 바뀐이름` 형태로 `detail`에 둔다.
 */
export function formatImportCsv(report: ImportReport): string {
  const rows = ["section,line,item,detail"];
  rows.push(...report.renamed_tags.map((r) => ["renamed", String(r.line), r.original, `${r.original} -> ${r.resolved}`].map(csvCell).join(",")));
  for (const [section, items] of [["modified", report.modified_items], ["ignored", report.ignored_items], ["unreadable", report.unreadable_items]] as const) {
    rows.push(...items.map((it) => [section, String(it.line), it.item, it.detail].map(csvCell).join(",")));
  }
  return rows.join("\r\n") + "\r\n";
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

/** UTC 밀리초를 현재 언어에 맞는 날짜·시각 문자열로. */
export function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString(localeTag());
}
