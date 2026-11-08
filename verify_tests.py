#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Rust 测试基线 —— 一键复现（零第三方依赖，Python 3.8+）。

为什么需要它：出包脚本 `build_main.bat` / `build_win7.bat` 成功后会把整个
`target\\` 删掉（项目约定：保持目录只有 17 MB）。因此「53 项测试全绿」很容易
变成一个只有历史记录、当下无法一键复现的结论。本脚本把这个结论重新变成
随时可复跑的事实。

用法（在 app_rust\\frameanalyzer\\ 目录下）：
    python verify_tests.py                     # 只跑主版本 1.98，跑完清理 target
    python verify_tests.py --keep-target       # 保留 target，便于连续复跑/调试
    python verify_tests.py --all               # 串行跑 主版本 + Win7 版 两套
    python verify_tests.py --expect-passed 53  # 断言通过数必须等于基线值

双击入口：同目录 `test_baseline.bat`。

设计要点：
- 源码指纹：对 Cargo.toml / Cargo.lock / build.rs / src/**/*.rs / tests/**/*.rs
  求 sha256，让「N 项全绿」绑定到具体源码，换过代码后报告里的指纹会变。
- 两套环境严格**串行**：并行全量编译曾导致 17 分钟挂死（见项目日志第十六/十七批）。
- 失败时保留 `target\\`：沿用出包脚本的失败保护，方便直接复跑排查。
- 成功且未指定 --keep-target 时清理 `target\\`，不破坏「目录保持精简」的约定。
"""
import argparse
import hashlib
import pathlib
import re
import shutil
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent          # app_rust/frameanalyzer
WIN7 = HERE.parent / "frameanalyzer-win7"

FINGERPRINT_FILES = ("Cargo.toml", "Cargo.lock", "build.rs")
RESULT_RE = re.compile(
    r"test result:\s*(\w+)\.\s*(\d+) passed;\s*(\d+) failed;\s*(\d+) ignored;")


def src_fingerprint(base):
    """把参与测试的源码汇总成一个 sha256，返回 (文件数, 短指纹)。"""
    items = []
    for rel in FINGERPRINT_FILES:
        p = base / rel
        if p.is_file():
            items.append((rel, p))
    for sub in ("src", "tests"):
        d = base / sub
        if d.is_dir():
            for p in sorted(d.rglob("*.rs")):
                items.append((str(p.relative_to(base)), p))
    h = hashlib.sha256()
    for rel, p in sorted(items):
        h.update(rel.encode("utf-8"))
        h.update(p.read_bytes())
    return len(items), h.hexdigest()[:16]


def toolchain(base, prefix):
    """读取工具链版本；读不到不致命，只显示 unknown。"""
    cmd = (prefix + ["--version"]) if prefix else ["cargo", "--version"]
    try:
        r = subprocess.run(cmd, cwd=str(base), text=True,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           timeout=30)
        return (r.stdout or "").strip().splitlines()[0]
    except Exception:
        return "unknown"


def run_cargo(cmd, base):
    """实时透传 cargo 输出（编译要几分钟，不能等到结束才打印），同时收集用于解析。"""
    print("  $ " + " ".join(cmd), flush=True)
    t0 = time.time()
    try:
        proc = subprocess.Popen(cmd, cwd=str(base), text=True,
                                stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, bufsize=1)
    except FileNotFoundError:
        print("  [ERROR] 找不到 cargo，请确认已安装 Rust 并在 PATH 中。", flush=True)
        return 127, "", time.time() - t0
    lines = []
    for line in proc.stdout:
        lines.append(line)
        print("    " + line.rstrip(), flush=True)
    proc.wait()
    return proc.returncode, "".join(lines), time.time() - t0


def parse_results(out):
    """累加所有 'test result:' 行（--lib 通常只有一行，多 target 时也能汇总）。"""
    passed = failed = ignored = 0
    found = False
    for line in out.splitlines():
        m = RESULT_RE.search(line)
        if m:
            found = True
            passed += int(m.group(2))
            failed += int(m.group(3))
            ignored += int(m.group(4))
    return found, passed, failed, ignored


def clean_target(base):
    t = base / "target"
    if not t.is_dir():
        return
    try:
        shutil.rmtree(t)
        print("  已清理 target\\（保持项目目录精简）", flush=True)
    except Exception as e:
        print("  [WARN] target\\ 清理失败：%s（可手动删，不影响测试结论）" % e, flush=True)


def main():
    ap = argparse.ArgumentParser(description="Rust 测试基线一键复现")
    ap.add_argument("--keep-target", action="store_true",
                    help="跑完保留 target\\，便于连续复跑")
    ap.add_argument("--all", action="store_true",
                    help="串行追加跑 Win7 版（1.77.2）的测试")
    ap.add_argument("--expect-passed", type=int, default=None,
                    help="断言通过数等于该值（防止测试被误删却无人发现）")
    args = ap.parse_args()

    # 主版本固定 +stable；Win7 版由它自己的 rust-toolchain.toml 切到 1.77.2
    targets = [("MAIN / 1.98", HERE, ["cargo", "+stable", "test", "--lib"])]
    if args.all:
        if not WIN7.is_dir():
            print("[ERROR] 找不到 Win7 目录：%s" % WIN7)
            return 1
        targets.append(("WIN7 / 1.77.2", WIN7, ["cargo", "test", "--lib"]))

    print("=" * 62)
    print("  Rust 测试基线  |  %s  |  %d 套环境（严格串行）"
          % (time.strftime("%Y-%m-%d %H:%M:%S"), len(targets)))
    print("=" * 62, flush=True)

    results = []
    for label, base, cmd in targets:
        nfiles, fp = src_fingerprint(base)
        print("\n[%s]" % label)
        print("  目录      : %s" % base)
        print("  工具链    : %s" % toolchain(base, cmd[:2] if cmd[1].startswith("+") else cmd[:1]))
        print("  源码指纹  : %s  (%d 个文件)" % (fp, nfiles))

        rc, out, secs = run_cargo(cmd, base)
        found, passed, failed, ignored = parse_results(out)

        reasons = []
        if rc != 0:
            reasons.append("cargo 退出码 %d" % rc)
        if not found:
            reasons.append("未找到 test result 行（编译失败或参数有误）")
        if failed:
            reasons.append("%d 项失败" % failed)
        if args.expect_passed is not None and passed != args.expect_passed:
            reasons.append("通过数 %d ≠ 基线 %d" % (passed, args.expect_passed))
        ok = not reasons

        print("  结果      : %s  | %d passed / %d failed / %d ignored | %.1fs"
              % ("OK" if ok else "FAILED", passed, failed, ignored, secs))
        if reasons:
            print("  原因      : " + "; ".join(reasons))
            print("  target\\ 保留，便于直接复跑排查。")
        elif not args.keep_target:
            clean_target(base)

        results.append((label, ok, passed, failed, ignored, secs, fp))

    print("\n" + "=" * 62)
    print("  汇总")
    print("=" * 62)
    print("  %-16s %-8s %8s %8s %9s  %s" %
          ("环境", "结论", "通过", "失败", "耗时", "源码指纹"))
    for label, ok, passed, failed, ignored, secs, fp in results:
        print("  %-16s %-8s %8d %8d %8.1fs  %s" %
              (label, "PASS" if ok else "FAIL", passed, failed, secs, fp))
    all_ok = all(r[1] for r in results)
    print("\n  BASELINE %s" % ("PASS" if all_ok else "FAILED"))
    if not all_ok:
        print("  提示：先修源码再复跑；target\\ 已保留，第二次编译会快很多。")
    print("=" * 62, flush=True)
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
