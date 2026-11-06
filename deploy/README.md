# 서비스 등록과 롤백 (SQ-O04)

묶음(`nuriscada-<버전>-<운영체제>-<아키텍처>`)을 버전별로 나란히 두고 `current` 심볼릭 링크로 가리킨다.
이전 버전 묶음을 지우지 않으면 롤백은 링크만 되돌리면 된다.

## 설치 (Linux, systemd)

```sh
sudo useradd --system --home-dir /var/lib/nuriscada --create-home nuriscada
sudo mkdir -p /opt/nuriscada /etc/nuriscada
sudo tar -xzf nuriscada-0.1.0-alpha.1-linux-x86_64.tar.gz -C /opt/nuriscada
sudo ln -sfn /opt/nuriscada/nuriscada-0.1.0-alpha.1-linux-x86_64 /opt/nuriscada/current
sudo cp /opt/nuriscada/current/examples/demo.toml /etc/nuriscada/project.toml
# DATA_DIR 경로와 수신 주소를 현장에 맞게 고친다. 외부 노출은 --allow-external 과 HTTPS 프록시가 필요하다.
sudo cp deploy/nuriscada.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nuriscada
```

최초 한 번은 관리자를 만든다(같은 설정·데이터 경로로 실행해야 한다).

```sh
sudo -u nuriscada /opt/nuriscada/current/bin/nuriscada-server \
  --config /etc/nuriscada/project.toml --data-dir /var/lib/nuriscada \
  --init-admin admin
```

## 업데이트

1. 새 버전 묶음을 옆에 푼다 (기존 디렉터리는 그대로 둔다).
2. `SHA256SUMS`로 묶음 해시를 확인한다.
3. `ln -sfn`으로 `current`를 새 버전으로 옮기고 `systemctl restart nuriscada`.
4. `/api/v1/status`와 화면을 확인한다.

## 롤백

1. `ln -sfn`으로 `current`를 이전 버전으로 되돌린다.
2. `systemctl restart nuriscada`.
3. DB 파일 형식은 버전과 함께 바뀌지 않으므로(바뀌면 릴리스 노트에 적는다) 데이터 디렉터리는 그대로 둔다.

## 그 밖의 환경

- macOS: launchd plist를 직접 등록한다(샘플은 제공하지 않는다). 같은 원칙(버전별 디렉터리 + `current` 링크)을 쓴다.
- Windows: 서비스 등록기(NSSM 등)를 직접 쓴다. 묶음의 `.zip`을 버전별 폴더에 푼다.
- 자동 업데이트·설치 프로그램은 없다. 수동 패키지 교체가 업데이트 경로다(SQ-O06).
