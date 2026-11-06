#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright NuriScada contributors
# 무인 설치: 묶음을 풀고 systemd에 등록한다. 대화형 입력이 없다.
# 사용: sudo deploy/install.sh --archive dist/<묶음>.tar.gz \
#         --admin admin --password-env NURISCADA_ADMIN_PASSWORD [--prefix /opt/nuriscada]
# 관리 비밀번호는 환경 변수로만 받는다(명령줄·파일에 적지 않는다).
set -euo pipefail

PREFIX=/opt/nuriscada
CONFIG=/etc/nuriscada/project.toml
DATA_DIR=/var/lib/nuriscada
SERVICE_USER=nuriscada
ARCHIVE=""
ADMIN=""
PASSWORD_ENV=""
DRY_RUN=0

usage() {
  echo "사용: $0 --archive <묶음> --admin <계정> --password-env <변수> [--prefix <경로>] [--dry-run]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --archive) ARCHIVE="${2:?}"; shift 2 ;;
    --admin) ADMIN="${2:?}"; shift 2 ;;
    --password-env) PASSWORD_ENV="${2:?}"; shift 2 ;;
    --prefix) PREFIX="${2:?}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) usage ;;
  esac
done
[ -n "$ARCHIVE" ] && [ -n "$ADMIN" ] && [ -n "$PASSWORD_ENV" ] || usage
[ -f "$ARCHIVE" ] || { echo "묶음이 없다: $ARCHIVE" >&2; exit 1; }
if [ -z "${!PASSWORD_ENV:-}" ]; then echo "비밀번호 환경 변수가 비었다: $PASSWORD_ENV" >&2; exit 1; fi

run() {
  "$@"
}

if [ "$DRY_RUN" = 1 ]; then
  cat <<EOF
묶음 풀기: $ARCHIVE -> \$PREFIX/<버전>
current 링크 갱신, 설정 복사(없을 때만): $CONFIG
deploy/nuriscada.service 설치, daemon-reload, enable --now nuriscada
nuriscada-server --init-admin $ADMIN (비밀번호는 $PASSWORD_ENV 에서 읽음)
EOF
  exit 0
fi
# 묶음 최상위 디렉터리 하나를 버전 디렉터리로 옮긴다.
TOP="$(find "$WORK" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
[ -n "$TOP" ] || { echo "묵음 안에 디렉터리가 없다" >&2; exit 1; }
DEST="$PREFIX/$(basename "$TOP")"

run mkdir -p "$PREFIX" /etc/nuriscada "$DATA_DIR"
run id "$SERVICE_USER" >/dev/null 2>&1 || run useradd --system --home-dir "$DATA_DIR" --create-home "$SERVICE_USER"
run rm -rf "$DEST"
run mv "$TOP" "$DEST"
run ln -sfn "$DEST" "$PREFIX/current"
trap - EXIT
rm -rf "$WORK"
if [ ! -f "$CONFIG" ]; then
  run cp "$DEST/examples/demo.toml" "$CONFIG"
fi
run cp deploy/nuriscada.service /etc/systemd/system/nuriscada.service
run systemctl daemon-reload
run systemctl enable --now nuriscada
if [ "$DRY_RUN" = 1 ]; then
  echo "+ nuriscada-server --init-admin $ADMIN (비밀번호는 $PASSWORD_ENV 에서 읽음)"
else
  # 이미 관리자가 있으면 넘어간다.
  printenv "$PASSWORD_ENV" | run sudo -u "$SERVICE_USER" "$PREFIX/current/bin/nuriscada-server" \
    --config "$CONFIG" --data-dir "$DATA_DIR" --init-admin "$ADMIN" --password-stdin \
    || true
fi
echo "설치 끝: $DEST"
