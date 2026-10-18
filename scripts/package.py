#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright NuriScada contributors
"""릴리스 묶음을 만든다. 표준 라이브러리만 쓴다(Python 3.9 이상).

사용:
  cargo build --locked --release -p nuriscada-server -p nuriscada-sim-modbus
  (cd web && npm ci && npm run build)
  python3 scripts/package.py [--version 0.1.0-alpha.1] [--out dist]

묶음 구조(압축 파일 안의 최상위 폴더 `nuriscada-<버전>-<OS>-<CPU>/`):
  bin/nuriscada-server, bin/nuriscada-sim-modbus
  web/            웹 화면 빌드 결과(서버가 실행 파일 옆에서 자동으로 찾는다)
  examples/       시연 프로젝트
  docs/           사용자 문서
  LICENSE, NOTICE, SAFETY.md, SECURITY.md, THIRD_PARTY_NOTICES.txt, README.txt
묶음과 함께 `SHA256SUMS`를 쓴다.
"""
import argparse
import hashlib
import json
import platform
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(cmd, **kw):
    return subprocess.run(cmd, cwd=ROOT, check=True, text=True, capture_output=True, **kw).stdout


def workspace_version():
    meta = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1"]))
    for p in meta["packages"]:
        if p["name"] == "nuriscada-server":
            return p["version"]
    raise SystemExit("nuriscada-server 패키지를 찾지 못했다")


def os_arch():
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    machine = platform.machine().lower()
    arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "x86_64", "amd64": "x86_64"}.get(machine, machine)
    return system, arch


def third_party_notices():
    """실제로 배포 바이너리에 들어가는 Rust 크레이트와 웹 번들의 라이선스 고지를 모은다."""
    closure = set()
    for pkg in ("nuriscada-server", "nuriscada-sim-modbus"):
        out = run(["cargo", "tree", "-p", pkg, "-e", "normal", "--prefix", "none", "--locked", "--format", "{p}"])
        for line in out.splitlines():
            parts = line.split()
            if len(parts) >= 2 and not parts[0].startswith("nuriscada-"):
                closure.add((parts[0], parts[1].lstrip("v")))
    meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked"]))
    by_key = {(p["name"], p["version"]): p for p in meta["packages"]}
    lines = [
        "NuriScada 서드파티 고지",
        "=" * 24,
        "이 묶음의 실행 파일과 웹 화면에 포함된 외부 구성 요소와 그 라이선스다.",
        "NuriScada 자체는 Apache License 2.0이다(LICENSE).",
        "",
    ]
    missing = []
    for name, version in sorted(closure):
        pkg = by_key.get((name, version))
        if pkg is None:
            missing.append(f"{name} {version}")
            continue
        lines.append("-" * 72)
        lines.append(f"{name} {version}")
        lines.append(f"라이선스: {pkg.get('license') or '(명시 없음)'}")
        if pkg.get("repository"):
            lines.append(f"저장소: {pkg['repository']}")
        base = Path(pkg["manifest_path"]).parent
        texts = sorted(p for p in base.iterdir() if p.is_file() and p.name.upper().startswith(("LICENSE", "LICENCE", "NOTICE", "COPYING")))
        for t in texts:
            lines.append("")
            lines.append(f"[{t.name}]")
            lines.append(t.read_text(encoding="utf-8", errors="replace").strip())
        if not texts:
            lines.append("(소스에 라이선스 파일이 없다. 위 라이선스 식별자를 따른다)")
        lines.append("")
    # 웹 화면에 번들되는 런타임(Svelte)
    svelte = ROOT / "web" / "node_modules" / "svelte"
    if (svelte / "package.json").is_file():
        pj = json.loads((svelte / "package.json").read_text(encoding="utf-8"))
        lines.append("-" * 72)
        lines.append(f"svelte {pj['version']} (웹 화면에 번들됨)")
        lines.append(f"라이선스: {pj.get('license')}")
        for f in sorted(svelte.iterdir()):
            if f.is_file() and f.name.upper().startswith("LICENSE"):
                lines.append("")
                lines.append(f.read_text(encoding="utf-8", errors="replace").strip())
        lines.append("")
    else:
        missing.append("svelte (web/node_modules가 없다)")
    if missing:
        raise SystemExit("고지를 만들지 못한 구성 요소: " + ", ".join(missing))
    return "\n".join(lines) + "\n"


READ_ME = """NuriScada {version} ({system}-{arch})

초기 공개(얼리 액세스) 버전입니다. 실제 PLC로 검증하지 않았고 MODBUS TCP만 지원합니다.
인명·설비 안전 기능에 단독으로 쓰지 마세요(SAFETY.md).

시작하기:
  1) docs/user/01-시작하기.md 를 읽으세요.
  2) bin/nuriscada-sim-modbus --demo            (시험용 장치 시뮬레이터)
  3) bin/nuriscada-server --config examples/demo.toml --init-admin admin
  4) bin/nuriscada-server --config examples/demo.toml
  5) 브라우저에서 http://127.0.0.1:8080/

라이선스: Apache-2.0 (LICENSE). 서드파티 고지: THIRD_PARTY_NOTICES.txt
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default=None)
    ap.add_argument("--out", default="dist")
    ap.add_argument("--target-dir", default="target/release")
    args = ap.parse_args()

    version = args.version or workspace_version()
    system, arch = os_arch()
    exe = ".exe" if system == "windows" else ""
    name = f"nuriscada-{version}-{system}-{arch}"
    out = ROOT / args.out
    stage = out / name
    if stage.exists():
        shutil.rmtree(stage)
    (stage / "bin").mkdir(parents=True)

    for b in ("nuriscada-server", "nuriscada-sim-modbus"):
        src = ROOT / args.target_dir / (b + exe)
        if not src.is_file():
            raise SystemExit(f"{src}가 없다. 먼저 cargo build --release로 빌드하라")
        shutil.copy2(src, stage / "bin" / src.name)

    dist = ROOT / "web" / "dist"
    if not (dist / "index.html").is_file():
        raise SystemExit("web/dist가 없다. 먼저 (cd web && npm ci && npm run build)를 실행하라")
    shutil.copytree(dist, stage / "web")
    shutil.copytree(ROOT / "examples", stage / "examples")
    shutil.copytree(ROOT / "docs" / "user", stage / "docs" / "user")
    for f in ("LICENSE", "NOTICE", "SAFETY.md", "SECURITY.md"):
        shutil.copy2(ROOT / f, stage / f)
    (stage / "THIRD_PARTY_NOTICES.txt").write_text(third_party_notices(), encoding="utf-8")
    (stage / "README.txt").write_text(READ_ME.format(version=version, system=system, arch=arch), encoding="utf-8")

    if system == "windows":
        archive = out / f"{name}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
            for p in sorted(stage.rglob("*")):
                z.write(p, Path(name) / p.relative_to(stage))
    else:
        archive = out / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as t:
            t.add(stage, arcname=name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    sums = out / "SHA256SUMS"
    existing = [l for l in (sums.read_text().splitlines() if sums.exists() else []) if not l.endswith("  " + archive.name)]
    sums.write_text("\n".join(existing + [f"{digest}  {archive.name}"]) + "\n")
    print(f"묶음: {archive} ({archive.stat().st_size / 1_048_576:.1f} MiB)")
    print(f"SHA-256: {digest}")


if __name__ == "__main__":
    sys.exit(main())
