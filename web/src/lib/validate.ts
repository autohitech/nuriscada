// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import type { ProjectSpec } from "./types";

export type ProjectIssue =
  | { code: "empty-tag-name"; tag: string }
  | { code: "dup-tag-name"; tag: string }
  | { code: "unknown-device"; tag: string; device: string }
  | { code: "dup-widget-id"; screen: string; id: string }
  | { code: "dup-screen-name"; screen: string };

/** 저장 전에 화면에서 바로 잡을 수 있는 오류를 찾는다(순수 함수라 시험할 수 있다). */
export function validateProjectSpec(spec: ProjectSpec): ProjectIssue[] {
  const issues: ProjectIssue[] = [];
  const devices = new Set(spec.device.map((d) => d.name));
  const names = new Set<string>();
  for (const t of spec.tag) {
    if (t.name.trim() === "") {
      issues.push({ code: "empty-tag-name", tag: t.name });
      continue;
    }
    if (names.has(t.name)) issues.push({ code: "dup-tag-name", tag: t.name });
    else names.add(t.name);
    if (!devices.has(t.device)) {
      issues.push({ code: "unknown-device", tag: t.name, device: t.device });
    }
  }
  const screens = new Set<string>();
  for (const s of spec.screen ?? []) {
    if (screens.has(s.name)) issues.push({ code: "dup-screen-name", screen: s.name });
    else screens.add(s.name);
    const ids = new Set<string>();
    for (const w of s.widget) {
      if (ids.has(w.id)) issues.push({ code: "dup-widget-id", screen: s.name, id: w.id });
      else ids.add(w.id);
    }
  }
  return issues;
}
