#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright NuriScada contributors
"""Linux·macOS 장기 부하 시험과 RSS·파일 디스크립터 추세 기록."""

import argparse
import datetime
import hashlib
import json
import pathlib
import platform
import shutil
import subprocess
import time


def write_json(path, data):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def resources(pid):
    rss = int(subprocess.check_output(
        ["ps", "-o", "rss=", "-p", str(pid)], text=True, timeout=10).strip())
    if platform.system() == "Linux":
        descriptors = len(list(pathlib.Path(f"/proc/{pid}/fd").iterdir()))
    else:
        output = subprocess.check_output(
            ["/usr/sbin/lsof", "-a", "-p", str(pid), "-F", "f"],
            text=True, timeout=10)
        descriptors = sum(line.startswith("f") and line[1:].isdigit()
                          for line in output.splitlines())
    return {"rss_kib": rss, "file_descriptors": descriptors}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path,
                        default=pathlib.Path("target/release/nuriscada-bench"))
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--duration-s", type=int, default=72 * 60 * 60)
    parser.add_argument("--sample-s", type=float, default=60)
    args = parser.parse_args()
    if args.duration_s < 1 or args.sample_s <= 0:
        parser.error("duration-s와 sample-s는 양수여야 한다")
    if platform.system() not in ("Linux", "Darwin"):
        parser.error("자원 기록은 Linux·macOS에서만 지원한다")
    source = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    binary = output / "nuriscada-bench"
    shutil.copy(source, binary)
    command = [str(binary), "--tags", "10000", "--devices", "10", "--clients", "50",
               "--scan-ms", "100", "--change-pct", "10", "--duration-s",
               str(args.duration_s), "--check", "--json"]
    metadata = {
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(), "machine": platform.machine(),
        "command": command, "sample_s": args.sample_s,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "git_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "git_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True)),
        "state": "running", "stability_pass": None,
    }
    started = time.monotonic()
    samples = 0
    errors = 0
    reason = None
    with (output / "stdout.json").open("w") as stdout, \
            (output / "stderr.log").open("w") as stderr, \
            (output / "samples.jsonl").open("w", buffering=1) as trace:
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        metadata["pid"] = process.pid
        write_json(output / "status.json", metadata)
        print(f"시작: PID {process.pid}, 기록 {output}", flush=True)
        try:
            while process.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed > args.duration_s + 120:
                    reason = "측정 시간이 끝난 뒤 120초 동안 종료하지 않았다"
                    break
                sample = {"elapsed_s": round(elapsed, 3)}
                try:
                    sample.update(resources(process.pid))
                    samples += 1
                except (OSError, ValueError, subprocess.SubprocessError) as error:
                    if process.poll() is not None:
                        break
                    sample["error"] = str(error)
                    errors += 1
                trace.write(json.dumps(sample, ensure_ascii=False) + "\n")
                try:
                    process.wait(timeout=args.sample_s)
                except subprocess.TimeoutExpired:
                    pass
        except KeyboardInterrupt:
            reason = "사용자 중단"
        except Exception as error:
            reason = f"기록 도구 오류: {error}"
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
    try:
        report = json.loads((output / "stdout.json").read_text())
    except (OSError, ValueError):
        report = None
    performance_pass = process.returncode == 0 and report is not None and report.get("pass") is True
    metadata.update({
        "state": "completed" if reason is None else "interrupted",
        "ended_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "elapsed_s": round(time.monotonic() - started, 3),
        "exit_code": process.returncode, "reason": reason,
        "resource_samples": samples, "resource_errors": errors,
        "performance_pass": performance_pass,
        "stability_note": "자원 추세 검토 전에는 누수 없음으로 판정하지 않는다",
    })
    write_json(output / "status.json", metadata)
    print(json.dumps(metadata, ensure_ascii=False), flush=True)
    return 0 if performance_pass and samples > 0 and errors == 0 and reason is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
