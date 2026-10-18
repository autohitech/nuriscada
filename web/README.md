# web

NuriScada 웹 UI. Svelte 5 + TypeScript + Vite ([결정 0014](../docs/decisions/0014-web-ui-svelte.md)).

```sh
npm ci
npm run dev       # 개발 서버(/api는 127.0.0.1:8080으로 넘김)
npm run check     # svelte-check
npm test          # vitest
npm run build     # dist/ (서버가 --web-dir 또는 web/dist로 제공)
npm run licenses  # 의존성 라이선스 검사
```

- 외부 CDN·글꼴·원격 스크립트를 쓰지 않는다(오프라인 동작, 결정 0003).
- 화면 문자열은 `src/lib/strings.ts`에 모았다(다국어는 다음 조각).
- 디자인 토큰은 `src/styles/tokens.css`(다크 기본, 라이트·고대비).
