# 메인테이너 가이드: 공개 저장소 설정

공개된 저장소를 처음 설정할 때 점검할 것입니다. 저장소 관리자(Admin) 권한이 필요합니다. 화면 이름은 GitHub 화면 기준이며 바뀔 수 있습니다.

## 1. 비공개 취약점 신고 켜기 (필수)

> **상태: 켜짐**(2026-10-18, `gh api repos/autohitech/nuriscada/private-vulnerability-reporting`가 `{"enabled":true}`). 아래 확인 단계(시험 신고, 알림 담당자 2명)는 아직 하지 않았다.

[SECURITY.md](../SECURITY.md)가 신고자에게 "Security 탭의 Report a vulnerability"를 쓰라고 안내합니다. 이 기능을 켜지 않으면 비공개로 받을 곳이 없어서 신고자가 공개 이슈로 올릴 수 있습니다.

**웹 화면**
1. 저장소 → **Settings** → **Security** 아래 **Code security**(일부 화면에서는 *Advanced Security*).
2. **Private vulnerability reporting** → **Enable**.
3. 조직 전체에 기본값을 주려면 조직 **Settings → Code security**의 *Private vulnerability reporting*을 새 저장소에 자동으로 켜도록 설정합니다.

**명령줄** (GitHub CLI, 관리자 권한)
```sh
gh api -X PUT repos/autohitech/nuriscada/private-vulnerability-reporting
gh api repos/autohitech/nuriscada/private-vulnerability-reporting   # {"enabled":true} 이어야 한다
```

**확인**
1. 저장소 협력자가 아닌 계정(또는 로그아웃한 창)으로 저장소의 **Security** 탭에 들어가 **Report a vulnerability** 버튼이 보이는지 확인합니다.
2. 시험 신고를 하나 올려 메인테이너에게 알림이 오는지 확인하고, 임시 보안 권고(Draft advisory)를 닫습니다.
3. 알림 담당자를 **최소 2명** 정합니다([결정 0008](decisions/0008-security-response-policy.md)). 각 담당자가 저장소를 *Watch → Custom → Security alerts*로 두고, 관리(Maintain) 이상 권한이 있는지 확인합니다.
4. 접수 확인 7일, 수정 목표 90일 목표를 지킬 수 있도록 담당자와 대리자를 `SECURITY.md` 또는 비공개 문서에 적어 둡니다.

## 2. 함께 켜면 좋은 보안 기능 (공개 저장소는 무료)

| 설정 | 위치 | 이유 |
|---|---|---|
| Dependabot alerts, Dependabot security updates | Settings → Code security | 의존성 취약점을 알려 주고 수정 PR을 만든다 |
| Secret scanning, Push protection | 같은 곳 | 비밀 값이 올라가는 것을 막는다 |
| 코드 스캔(CodeQL) | Security → Code scanning | 선택. Rust·TypeScript를 정적 점검한다 |
| 보안 정책 표시 | `SECURITY.md`가 있으면 자동 | 신고 안내가 Security 탭에 보인다 |

## 3. `main` 브랜치 보호

저장소가 공개되어 브랜치 보호(또는 Rulesets)를 쓸 수 있습니다. [결정 0009](decisions/0009-github-operations.md)는 "보호 없음"으로 정했으므로 이 설정은 **결정을 바꾸는 일**입니다. 바꾼다면 다음을 권합니다.

- Settings → Rules → Rulesets(또는 Branches → Branch protection rules)에서 `main`에 **Require a pull request before merging**, **Require status checks to pass**를 켠다.
- 필수 검사: `rust (ubuntu-latest)`, `rust (macos-latest)`, `rust (windows-latest)`, `web (node)`, `의존성 라이선스·취약점 검사`.
- **Block force pushes**와 삭제 금지를 켠다. 관리자 예외가 필요하면 명시한다.
- 외부 기여가 시작되면 리뷰 1명 요구와 DCO 검사(DCO 앱 또는 워크플로)를 함께 켠다.

## 4. 커밋에 노출되는 이메일

커밋의 작성자 이메일은 공개 이력에 그대로 보입니다(지금 이력의 모든 커밋은 개인 이메일입니다). 이후 커밋부터 숨기려면:

1. GitHub 계정 **Settings → Emails**에서 *Keep my email addresses private*와 *Block command line pushes that expose my email*을 켠다.
2. 로컬에서 noreply 주소를 쓴다: `git config user.email "<숫자ID>+<사용자명>@users.noreply.github.com"` (주소는 Emails 설정에 표시됨).
3. **이미 올라간 커밋은 바뀌지 않는다.** 이력의 이메일을 지우려면 이력을 다시 써서 강제 푸시해야 하고, 포크와 캐시에는 남을 수 있으므로 필요성을 먼저 판단한다.

## 5. 공개 전환 조건 점검표

[결정 0007](decisions/0007-repository-release-and-ownership.md)의 8개 조건과 현재 상태(2026-10-18):

| # | 조건 | 상태 |
|---|---|---|
| 1 | 첫 동작 수직 조각 + 성능 측정 | 충족 |
| 2 | 이름 충돌 확인(OD-6) | 1차 조회 완료(충돌 없음). 공식 상표 데이터베이스 조회는 전문가 의뢰 필요 |
| 3 | 책임 제한·코덱 법무 확인(OD-8) | 검토 의뢰서 준비 완료, 답변 대기 |
| 4 | `SECURITY.md`와 비공개 취약점 신고 기능 | 문서 있음. **기능 활성화 완료**(2026-10-18, `gh api`로 `enabled: true` 확인). 시험 신고와 알림 담당자 2명 지정은 아직 확인하지 않았다 |
| 5 | 금지 패턴 검사와 이력 점검 | 완료(26개 커밋 전체 검사, 기존 제품 흔적 없음) |
| 6 | 의존성 라이선스 검사와 SBOM(CI) | 충족 |
| 7 | 안전 고지·보증 부인이 README 첫 화면에 있음 | 충족 |
| 8 | 소유자 최종 승인 | 완료(소유자 확인, 2026-10-18) |
