#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright NuriScada contributors
# 사용: scripts/visual.sh [--update | --review]
#   --update: 기준 스냅샷을 갱신한다(로컬에서 눈으로 확인한 뒤에만 커밋한다).
#   --review: 스냅샷을 항상 새로 쓰고 통과시킨다(CI에서 촬영본을 아티팩트로 남길 때 쓴다).
set -euo pipefail
cd "$(dirname "$0")/.."

UPDATE=""
if [ "${1:-}" = "--review" ]; then UPDATE="--update-snapshots"; fi
if [ "${1:-}" = "--update" ]; then UPDATE="--update-snapshots"; fi

PORT=18080
DATA="$(mktemp -d)"
trap 'rm -rf "$DATA"' EXIT

cargo build --locked --release -p nuriscada-server -p nuriscada-sim-modbus 2>&1 | tail -1
(cd web && npm ci --no-audit --no-fund >/dev/null 2>&1 && npm run build >/dev/null 2>&1)
./target/release/nuriscada-sim-modbus --listen 127.0.0.1:5020 &
SIM=$!
trap 'kill $SIM 2>/dev/null; rm -rf "$DATA"' EXIT
printf '%s\n' "visual-test-password-123" | ./target/release/nuriscada-server \
  --config examples/demo.toml --data-dir "$DATA" --init-admin visual --password-stdin >/dev/null
./target/release/nuriscada-server --config examples/demo.toml --data-dir "$DATA" \
  --listen "127.0.0.1:$PORT" --web-dir web/dist >/tmp/nuriscada-visual.log 2>&1 &
SERVER=$!
trap 'kill $SIM $SERVER 2>/dev/null; rm -rf "$DATA"' EXIT
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:$PORT/api/v1/status" >/dev/null 2>&1 && break
  sleep 0.2
done
(cd web && PLAYWRIGHT_BASE_URL="http://127.0.0.1:$PORT" npx playwright test $UPDATE)
