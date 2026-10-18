// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";
// 서버 소스를 그대로 읽어 코드 목록을 얻는다(번역이 빠지면 시험이 실패한다).
import serverSource from "../../../crates/server/src/error_code.rs?raw";
import { apiErrorMessage } from "./api";
import { setLocale } from "./i18n.svelte";
import { en } from "./locales/en";
import { ko } from "./locales/ko";

/** 서버 소스의 오류 코드 식별자(`Self::X => "a.b"`)를 읽는다. */
function serverCodes(): string[] {
  return [...serverSource.matchAll(/Self::\w+ => "([a-z_]+\.[a-z_]+)",/g)].map((m) => m[1] as string);
}

describe("server error codes", () => {
  it("finds the codes in the server source", () => {
    expect(serverCodes().length).toBeGreaterThanOrEqual(25);
  });

  it("every server code has a Korean and an English message, and vice versa", () => {
    const codes = serverCodes().sort();
    expect(Object.keys(ko.errors).sort()).toEqual(codes);
    expect(Object.keys(en.errors).sort()).toEqual(codes);
    for (const code of codes) {
      expect(ko.errors[code]?.length, code).toBeGreaterThan(0);
      expect(en.errors[code]?.length, code).toBeGreaterThan(0);
    }
  });

  it("English error messages contain no Hangul", () => {
    for (const [code, message] of Object.entries(en.errors)) {
      expect(/[가-힣]/.test(message), code).toBe(false);
    }
  });
});

describe("apiErrorMessage", () => {
  it("uses the current language for a known code", () => {
    setLocale("en", false);
    expect(apiErrorMessage({ code: "auth.required", error: "로그인이 필요하다" }, "x")).toBe("Sign-in is required.");
    setLocale("ko", false);
    expect(apiErrorMessage({ code: "auth.required", error: "로그인이 필요하다" }, "x")).toBe("로그인이 필요합니다.");
  });

  it("appends the server's detail without translating it", () => {
    setLocale("en", false);
    expect(apiErrorMessage({ code: "project.invalid", detail: "태그 이름이 중복된다" }, "x")).toBe(
      "The project settings are not valid. (태그 이름이 중복된다)",
    );
    setLocale("ko", false);
  });

  it("falls back to the server message, then to the fallback", () => {
    expect(apiErrorMessage({ code: "unknown.code", error: "서버 문구" }, "x")).toBe("서버 문구");
    expect(apiErrorMessage({ error: "서버 문구" }, "x")).toBe("서버 문구");
    expect(apiErrorMessage({}, "대체 문구")).toBe("대체 문구");
    expect(apiErrorMessage(null, "대체 문구")).toBe("대체 문구");
  });
});
