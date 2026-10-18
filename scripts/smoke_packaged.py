#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright NuriScada contributors
"""만든 배포 묶음을 풀어서 실제로 실행되는지 확인한다.

사용: python3 scripts/smoke_packaged.py dist
관리자 계정을 만들고 서버를 시작해 웹 화면(`/`)과 로그인 필요 응답(`/api/v1/status`가 401)을 확인한다.
"""
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    dist = Path(sys.argv[1] if len(sys.argv) > 1 else "dist")
    archives = sorted(list(dist.glob("*.tar.gz")) + list(dist.glob("*.zip")))
    if not archives:
        raise SystemExit("묶음 파일이 없다")
    archive = archives[-1]
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        if archive.suffix == ".zip":
            zipfile.ZipFile(archive).extractall(tmp)
        else:
            tarfile.open(archive).extractall(tmp)
        root = next(p for p in tmp.iterdir() if p.is_dir())
        exe = ".exe" if sys.platform == "win32" else ""
        server = root / "bin" / ("nuriscada-server" + exe)
        for need in ("README.txt", "LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.txt", "docs/user/01-시작하기.md", "web/index.html", "examples/demo.toml"):
            if not (root / need).is_file():
                raise SystemExit(f"묶음에 {need}가 없다")

        # 설정과 데이터는 임시 폴더에 둔다(묶음 안의 예제는 건드리지 않는다).
        config = tmp / "project.toml"
        port = free_port()
        config.write_text(
            f'[server]\nlisten = "127.0.0.1:{port}"\n\n[[device]]\nname = "d"\ndriver = "modbus-tcp"\naddress = "127.0.0.1:1"\n',
            encoding="utf-8",
        )
        init = subprocess.run(
            [str(server), "--config", str(config), "--init-admin", "admin", "--password-stdin"],
            input="smoke-test-password-123\n", text=True, capture_output=True, timeout=60,
        )
        if init.returncode != 0:
            raise SystemExit(f"관리자 만들기 실패: {init.stderr}")
        proc = subprocess.Popen([str(server), "--config", str(config)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.time() + 20
            html = None
            while time.time() < deadline:
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=2) as r:
                        html = r.read().decode("utf-8", "replace")
                        break
                except Exception:
                    if proc.poll() is not None:
                        raise SystemExit(f"서버가 끝났다: {proc.stderr.read()}")
                    time.sleep(0.3)
            if not html or "<div id=\"app\">" not in html:
                raise SystemExit("웹 화면(index.html)이 응답하지 않았다")
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/api/v1/status", timeout=3)
                raise SystemExit("로그인 없이 상태 API가 응답했다(인증이 빠졌다)")
            except urllib.error.HTTPError as e:
                if e.code != 401:
                    raise SystemExit(f"상태 API가 401이어야 하는데 {e.code}")
            print(f"묶음 실행 확인 통과: {archive.name}")
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()


if __name__ == "__main__":
    main()
