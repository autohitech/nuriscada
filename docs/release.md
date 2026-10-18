# 릴리스 절차 (관리자용)

초기 공개(얼리 액세스)는 `v0.x.y-alpha.N` 시험판으로 낸다. 코드 서명과 설치 프로그램이 없는 **압축 묶음**이다.

## 묶음 내용

`scripts/package.py`가 실행 파일(`nuriscada-server`, `nuriscada-sim-modbus`), 웹 화면 빌드(`web/`), 예제, 사용자 문서(`docs/user/`), `LICENSE`, `NOTICE`, `SAFETY.md`, `SECURITY.md`, 서드파티 고지(`THIRD_PARTY_NOTICES.txt`), `README.txt`를 한 묶음으로 만들고 `SHA256SUMS`를 쓴다. 서드파티 고지는 실제 배포 바이너리의 Rust 의존성 목록과 각 라이선스 파일, 번들된 웹 런타임에서 만든다. 라이선스 파일이 없는 구성 요소가 있으면 라이선스 식별자만 적는다.

## 로컬에서 묶음 만들기

```sh
(cd web && npm ci && npm run build)
cargo build --locked --release -p nuriscada-server -p nuriscada-sim-modbus
python3 scripts/package.py            # dist/ 에 묶음과 SHA256SUMS
python3 scripts/smoke_packaged.py dist  # 풀어서 서버가 뜨고 인증이 걸리는지 확인
```

## 릴리스 내기

1. **공개 전환 조건 확인**: [결정 0007](decisions/0007-repository-release-and-ownership.md)의 8개 조건(이름 상표 조회, 법무 검토, 보안 신고 기능 활성화, 이력 점검 등). 저장소가 비공개인 동안에는 릴리스를 내부 검토용 초안으로만 만든다.
2. 버전 올리기: 루트 `Cargo.toml`의 `[workspace.package] version`(예: `0.1.0-alpha.2`)과 `Cargo.lock`을 함께 갱신해 병합한다.
3. `main`에서 태그를 올린다: `git tag v0.1.0-alpha.2 && git push origin v0.1.0-alpha.2`. 태그 버전과 실행 파일 버전이 다르면 워크플로가 실패한다.
4. `release` 워크플로가 세 OS(Linux x86-64, Windows x86-64, macOS arm64)에서 웹 검사·시험·빌드, 묶음 만들기, **묶음 실행 확인**을 하고 **초안·시험판** 릴리스를 만든다.
5. 초안에서 확인한다: 파일 목록, `SHA256SUMS`, 릴리스 노트(`docs/release-notes-template.md`에 변경 내용을 적는다), 세 OS 묶음을 내려받아 `docs/user/01-시작하기.md`대로 한 번 실행해 본다.
6. 공개(Publish)는 사람이 누른다.

## 알려진 제약

- 코드 서명·공증이 없어 macOS·Windows에서 경고가 뜬다(사용자 문서에 안내).
- macOS는 arm64만, Linux·Windows는 x86-64만 만든다. 다른 조합은 아직 없다.
- GitHub Actions 사용 한도가 부족하면 macOS·Windows 작업이 시작되지 않는다. 한도를 확인하고 재실행한다.
- 자동 업데이트, 설치 프로그램, 서비스 등록은 없다.
