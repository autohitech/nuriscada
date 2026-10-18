# NuriScada

NuriScada는 Windows, Linux, macOS에서 동작하는 무료 오픈소스 SCADA 소프트웨어입니다. **현재 설계 단계**이며 실행할 수 있는 코드는 아직 없습니다.

- 라이선스: [Apache-2.0](LICENSE). 상업적 사용을 포함해 자유롭게 쓸 수 있습니다.
- **보증과 책임 없음**: 이 소프트웨어는 "있는 그대로" 제공되며 개발자는 사용으로 생기는 어떤 손해에도 책임을 지지 않습니다. 자세한 내용은 [LICENSE](LICENSE)와 [안전 고지](SAFETY.md)를 읽으세요.
- **안전 고지**: 인명이나 설비의 안전을 보장하는 용도(비상 정지, 안전 계전기 대체 등)에 단독으로 쓰지 마세요.

## 현재 문서

| 문서 | 내용 |
|---|---|
| [고품질 SCADA 요구사항](docs/requirements/scada-quality-requirements.md) | 그래픽·SVG, UI·UX 디자인 시스템, 실시간성, 알람, 보안, 호환성 요구 (초안) |
| [의존성·라이선스 정책](docs/requirements/dependency-license-policy.md) | 쓸 수 있는 라이선스, SBOM, 고지, 프로토콜 구현 원칙 (초안) |
| [결정 기록](docs/decisions/) | 확정한 결정 |

## 방향

- 처음부터 새로 만든다. 기존 제품의 코드와 자산을 가져오지 않는다.
- 기존 제품의 프로젝트 파일을 읽을 수 있게 한다 (쓰기는 선택).
- 장치 드라이버는 공개 규격에서 새로 작성한다. 대표 5계열(MODBUS·MELSEC·XGT·OMRON·S7)부터 시작한다.
- 지원 대상: Windows 10+, 주요 Linux, macOS 최신 두 버전, x86-64와 arm64.

기술 스택: Rust 코어·드라이버·서버 + 웹 UI([결정 기록](docs/decisions/0002-tech-stack-rust-core-web-ui.md)).

## 결정 현황

### 확정

| 항목 | 결정 | 근거 |
|---|---|---|
| 제품 이름 | NuriScada (기존 제품 이름은 쓰지 않음) | [결정 기록](docs/decisions/0011-project-name-nuriscada.md) |
| 라이선스 | Apache-2.0, 소스 공개, 상업적 사용 허용, 보증·책임 없음 | [결정 기록](docs/decisions/0001-license-apache-2.0.md) |
| 지원 대상 | Windows 10+, 주요 Linux, macOS 최신 두 버전, x86-64와 arm64 (임베디드 소형 기기는 1단계 제외) | 2026-10-18 |
| 드라이버 | 공개 규격에서 새로 작성, 대표 5계열 먼저 | 2026-10-18 |
| 기존 코드 | 가져오지 않고 명세에서 다시 작성 | 2026-10-18 |
| 기존 프로젝트 파일 | 읽기 필수, 쓰기 선택 | 2026-10-18 |
| 동작 기준선 (OD-10) | 관측 기준선 없이 예측으로 진행. 기대값 JSON만 이식, 시험 코드는 Rust로 새로 작성. 기존 제품과 동일하다고 주장하지 않음 | [결정 기록](docs/decisions/0010-behavior-baseline-prediction-only.md) |
| GitHub 운영 (OD-11) | 스쿼시 병합만, 병합 후 브랜치 자동 삭제, main 보호 없음(위험은 결정 기록 참고), 핵심 멤버만 쓰기·외부는 포크+PR | [결정 기록](docs/decisions/0009-github-operations.md) |
| 보안 대응 (OD-9) | 비공개 보안 신고, 접수 확인 7일·수정 목표 90일, 최신 주 버전의 최신+직전 마이너 라인 지원, 수정 후 조율 공개 | [결정 기록](docs/decisions/0008-security-response-policy.md) |
| 저장소 공개·소유 (OD-7) | 첫 동작 수직 조각까지 비공개, 소유자 autohitech 조직, 저작권 표시 "NuriScada contributors" | [결정 기록](docs/decisions/0007-repository-release-and-ownership.md) |
| 표준·보안 (OD-5) | 표준은 설계 참고(인증 목표 아님), 인터넷 공개는 읽기 전용, 로컬 계정 + 선택적 OIDC/LDAP, 모든 쓰기·설정 변경·로그인 감사 기록 | [결정 기록](docs/decisions/0006-standards-security-policy.md) |
| 성능 목표 (OD-4) | 태그 약 1만 개, 최소 스캔 주기 100 ms, 동시 브라우저 약 50명, 지터 주기의 20% 이내(소프트 실시간) | [결정 기록](docs/decisions/0005-performance-targets.md) |
| 디자인 방향 (OD-3) | 차분한 산업용, 다크 기본(라이트·고대비 제공), 데스크톱 편집기와 터치 런타임을 용도별로 설계 | [결정 기록](docs/decisions/0004-design-direction.md) |
| UI 형태 (OD-2) | 브라우저 UI 우선, 데스크톱 셸은 나중에. 로컬·LAN 원격·인터넷 공개·오프라인 설치 시나리오 지원 | [결정 기록](docs/decisions/0003-ui-delivery-browser-first.md) |
| 기술 스택 (OD-1) | 코어·드라이버·서버는 Rust, UI는 웹(TypeScript) | [결정 기록](docs/decisions/0002-tech-stack-rust-core-web-ui.md) |

### 열린 결정

아래는 아직 정하지 않았다. "제안"은 근거가 있는 기본안이며 확정이 아니다.

| 번호 | 결정할 것 | 제안 | 정하는 데 필요한 것 |
|---|---|---|---|
| OD-6 | 이름 상표·도메인 확인 | 이름은 NuriScada로 확정([결정 기록](docs/decisions/0011-project-name-nuriscada.md)). 상표 조회와 도메인 등록은 공개 전환 전에 한다 | 상표 데이터베이스 조회 |
| OD-8 | 책임 제한 문구의 법적 효력, 영상 코덱 라이선스 | 법무 확인 | 전문가 검토 |

## 기여

[CONTRIBUTING.md](CONTRIBUTING.md)를 읽어 주세요. 보안 문제는 [SECURITY.md](SECURITY.md)의 절차로 알려 주세요.
