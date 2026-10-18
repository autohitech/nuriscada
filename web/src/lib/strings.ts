// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

/** 화면 문자열. 다국어 지원(SQ-U09) 때 언어별 사전으로 바꿀 수 있게 한곳에 모았다. */

import type { AlarmEvent, AlarmState } from "./types";

export const ko = {
  appTitle: "NuriScada",
  connected: "연결됨",
  reconnecting: "연결 끊김 — 다시 연결하는 중",
  alarms: "알람",
  noAlarms: "활성 알람이 없습니다",
  moreAlarms: (n: number) => `외 ${n}건`,
  devicesSummary: (n: number, up: number) => `장치 ${n}개 중 ${up}개 연결`,
  activeAlarms: "활성 알람",
  recentAlarms: "최근 기록",
  noRecent: "기록이 없습니다",
  tags: "태그",
  filterPlaceholder: "태그 이름 검색",
  filterLabel: "태그 이름으로 검색",
  colName: "태그",
  colValue: "값",
  colState: "상태",
  colChanged: "변경 시각",
  colDevice: "장치",
  on: "켜짐",
  off: "꺼짐",
  noValue: "—",
  lastValue: "마지막 값",
  badQuality: "통신 불량",
  devices: "장치",
  scans: "스캔",
  jitter: "지터 99%",
  theme: "테마",
  themeDark: "어둡게",
  themeLight: "밝게",
  themeContrast: "고대비",
  noData: "표시할 태그가 없습니다",
  noMatch: "검색 결과가 없습니다",
  safetyNotice: "안전 기능(비상 정지 등)에 단독으로 사용하지 마세요.",
} as const;

export const alarmStateLabel: Record<AlarmState, string> = {
  NORMAL: "정상",
  LOLO: "하하한",
  LOW: "하한",
  HIGH: "상한",
  HIHI: "상상한",
  MIDDLE: "범위 안",
};

export const alarmEventLabel: Record<AlarmEvent, string> = {
  HIHI: "상상한",
  HIGH: "상한",
  LOW: "하한",
  LOLO: "하하한",
  CNF: "확인",
  RETURN: "복귀",
  DI_ON: "켜짐",
  DI_OFF: "꺼짐",
};
