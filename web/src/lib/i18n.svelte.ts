// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { en } from "./locales/en";
import { ko, type Messages } from "./locales/ko";

export type Locale = "ko" | "en";
export const LOCALES: readonly Locale[] = ["ko", "en"];
/** 언어 선택 목록에 보이는 이름(각 언어 자신의 이름). */
export const LOCALE_NAMES: Record<Locale, string> = { ko: "한국어", en: "English" };

const STORAGE_KEY = "nuriscada.locale";
const dictionaries: Record<Locale, Messages> = { ko, en };

/**
 * 저장된 선택이 있으면 그것을, 없으면 브라우저 언어 목록의 첫 지원 언어를 쓴다. 둘 다 없으면 영어.
 * (순수 함수라 시험할 수 있다.)
 */
export function detectLocale(languages: readonly string[], saved?: string | null): Locale {
  if (saved && (LOCALES as readonly string[]).includes(saved)) return saved as Locale;
  for (const lang of languages) {
    const base = lang.toLowerCase().split("-")[0];
    if (base === "ko") return "ko";
    if (base === "en") return "en";
  }
  return "en";
}

/** 지금 화면 문자열. 속성을 읽는 곳이 언어 바꿈에 반응한다. */
export const m = $state<Messages>({ ...ko });
export const i18n = $state<{ locale: Locale }>({ locale: "ko" });

/** 언어를 바꾼다. 선택은 브라우저에 저장한다(저장 못 해도 동작한다). */
export function setLocale(locale: Locale, persist = true): void {
  Object.assign(m, dictionaries[locale]);
  i18n.locale = locale;
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  if (persist) {
    try {
      localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // 저장하지 못해도 화면은 정상이다.
    }
  }
}

/** 시작할 때 한 번 부른다. */
export function initLocale(): void {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(STORAGE_KEY);
  } catch {
    // 저장소를 쓸 수 없으면 브라우저 언어를 쓴다.
  }
  const languages = typeof navigator !== "undefined" ? navigator.languages ?? [navigator.language] : [];
  setLocale(detectLocale(languages, saved), false);
}

/** 날짜·숫자 형식에 쓰는 로캘 이름. */
export function localeTag(): string {
  return i18n.locale === "ko" ? "ko-KR" : "en-US";
}

/** 알람 상태 이름. 모르는 값은 그대로 보인다. */
export function stateLabel(state: string): string {
  return m.alarmState[state] ?? state;
}

/** 알람 기록 종류 이름. 모르는 값은 그대로 보인다. */
export function eventLabel(event: string): string {
  return m.alarmEvent[event] ?? event;
}
