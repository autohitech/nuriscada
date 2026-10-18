// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

/** 서버 API v1의 형식(설계 0003). */

export type Quality = "good" | "bad";
export type AlarmState = "NORMAL" | "LOLO" | "LOW" | "HIGH" | "HIHI" | "MIDDLE";
export type AlarmEvent = "HIHI" | "HIGH" | "LOW" | "LOLO" | "CNF" | "RETURN" | "DI_ON" | "DI_OFF";

export interface TagSnapshot {
  name: string;
  device: string;
  kind: "analog" | "digital";
  value: number | boolean | null;
  quality: Quality;
  changed_ms: number;
  unit_text: string;
  alarm_state: AlarmState | null;
}

export interface AlarmRecord {
  seq: number;
  time_ms: number;
  tag: string;
  event: AlarmEvent;
  value: number;
}

export interface DeviceStatus {
  name: string;
  connected: boolean;
  round_trip_ms: number | null;
  requests_ok: number;
  requests_failed: number;
  scan_ms: number;
  scans: number;
  overruns: number;
  jitter_p99_us: number | null;
  jitter_max_us: number;
  last_error: string | null;
}

export interface StatusDoc {
  version: string;
  uptime_s: number;
  tag_count: number;
  devices: DeviceStatus[];
}

export type StreamMessage =
  | { type: "snapshot"; data: { tags: TagSnapshot[]; active: TagSnapshot[]; recent: AlarmRecord[] } }
  | { type: "tag"; data: TagSnapshot }
  | { type: "alarm"; data: AlarmRecord };
