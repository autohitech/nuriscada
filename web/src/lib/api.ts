// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { m } from "./i18n.svelte";

/** 서버 오류 응답 본문(설계 0003). `code`로 현재 언어의 문구를 찾는다. */
export interface ApiErrorBody {
  code?: string;
  error?: string;
  detail?: string;
}

/**
 * 오류 응답에서 사용자에게 보일 문구를 만든다.
 *
 * 1. `code`가 사전에 있으면 현재 언어 문구를 쓴다. `detail`(서버가 준 구체적 이유)이 있으면 붙인다.
 *    `detail`은 번역되지 않는다.
 * 2. 코드가 없거나 사전에 없으면 서버의 기본 문구(`error`)를 쓴다.
 * 3. 그것도 없으면 `fallback`을 쓴다.
 */
export function apiErrorMessage(body: ApiErrorBody | null | undefined, fallback: string): string {
  const translated = body?.code ? m.errors[body.code] : undefined;
  if (translated) return body?.detail ? m.errorDetailJoin(translated, body.detail) : translated;
  return body?.error ?? fallback;
}
