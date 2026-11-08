#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""校验主版本（stable 1.98 / Win10-11）产物，并从 exe 本身判定它是哪套包。

为什么需要它
------------
两套环境产出的 exe **都叫 frameanalyzer.exe**，只靠文件名和所在目录区分，
拷错包的后果是 Win7 上 0xc0000005。这个脚本不看文件名，直接读 PE 导入表：

* Rust 1.78+ 的 std 会静态导入 bcryptprimitives.dll(ProcessPrng) 和
  api-ms-win-core-synch-l1-2-0.dll(WaitOnAddress) —— 这是**编译器留下的指纹**；
* 1.77.2 + crt-static 构建既没有这两个 DLL，也没有 UCRT 动态依赖。

所以导入表是客观证据，不依赖任何人声称"这是哪套"。

用法
----
    python verify_release.py [exe路径] [--expect main|win7|any]

默认检查本目录下的 frameanalyzer.exe（打包脚本交付的成品）、期望 main。
退出码 0 = 判定与期望一致；1 = 不一致或解析失败（可直接用于 bat 的 errorlevel）。

    python verify_release.py ..\frameanalyzer-win7\frameanalyzer.exe --expect win7
    # 用它反查 Win7 包也能识别出来 —— 这是判定逻辑的交叉验证
"""

import datetime
import hashlib
import pathlib
import struct
import sys

# Rust 1.78+ std 特有的 Win8+ 硬依赖；出现即说明是"现代工具链"构建
WIN8_MARKERS = {"bcryptprimitives.dll", "api-ms-win-core-synch-l1-2-0.dll"}
UCRT_NAMES = {"vcruntime140.dll", "vcruntime140_1.dll", "ucrtbase.dll"}


def pe_imports(path):
    """直接解析 PE 导入表，返回 dll 名列表（不调用任何外部工具）。"""
    data = pathlib.Path(path).read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    nsec = struct.unpack_from("<H", data, pe + 6)[0]
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from("<H", data, opt)[0]
    dd = opt + (0x70 if magic == 0x20B else 0x60)
    imp_rva, _ = struct.unpack_from("<II", data, dd + 8)

    sec = opt + opt_size
    sections = []
    for i in range(nsec):
        s = sec + i * 40
        va, vsz = struct.unpack_from("<II", data, s + 12)
        raw, rsz = struct.unpack_from("<II", data, s + 20)
        sections.append((va, vsz, raw, rsz))

    def rva2off(rva):
        for va, vsz, raw, rsz in sections:
            if va <= rva < va + max(vsz, rsz):
                return raw + (rva - va)
        return None

    names, o = [], rva2off(imp_rva)
    while o is not None:
        _, _, _, name_rva = struct.unpack_from("<IIII", data, o)
        if name_rva == 0:
            break
        no = rva2off(name_rva)
        if no is None:
            break
        names.append(data[no:data.index(b"\0", no)].decode("latin1"))
        o += 20
    return names


def classify(names):
    low = [n.lower() for n in names]
    modern = bool(WIN8_MARKERS & set(low))
    dynamic_crt = any(n.startswith("api-ms-win-crt-") or n in UCRT_NAMES or
                      n.startswith("ucrtbase") for n in low)
    if modern and dynamic_crt:
        return "main", "stable 1.98.x / 动态 VC 运行库", "Win10 / Win11（不可用于 Win7）"
    if not modern and not dynamic_crt:
        return "win7", "1.77.2 / crt-static 静态运行库", "Win7 / Win8 / Win10 / Win11"
    return "unknown", "无法识别的组合", "未知"


def main():
    # 显式解析 --expect 及其取值，不能让 "main" 混进路径参数里
    raw = sys.argv[1:]
    args, expect, i = [], "main", 0
    while i < len(raw):
        a = raw[i]
        if a == "--expect":
            if i + 1 < len(raw):
                expect = raw[i + 1]
            i += 2
        elif a.startswith("--expect="):
            expect = a.split("=", 1)[1]
            i += 1
        elif a.startswith("--"):
            i += 1
        else:
            args.append(a)
            i += 1
    if expect not in ("main", "win7", "any"):
        print(f"  [ERROR] --expect 只能是 main / win7 / any，收到: {expect}")
        return 1

    path = pathlib.Path(args[0] if args else "frameanalyzer.exe")
    if not path.is_file():
        print(f"  [ERROR] 找不到产物: {path}")
        return 1

    try:
        names = pe_imports(path)
    except Exception as e:
        print(f"  [ERROR] PE 解析失败: {e}")
        return 1
    if not names:
        print("  [ERROR] 导入表为空，文件可能不是有效 PE")
        return 1

    variant, toolchain, target = classify(names)
    data = path.read_bytes()
    st = path.stat()
    built = datetime.datetime.fromtimestamp(st.st_mtime).strftime("%Y-%m-%d %H:%M:%S")

    print(f"  文件    : {path}")
    print(f"  大小    : {len(data):,} 字节")
    print(f"  构建时间: {built}")
    print(f"  SHA256  : {hashlib.sha256(data).hexdigest()}")
    print(f"  导入 DLL: {len(names)} 个")
    hits = sorted(WIN8_MARKERS & {n.lower() for n in names})
    print(f"  Win8+ 指纹: {', '.join(hits) if hits else '无'}")
    ucrt = [n for n in names if n.lower().startswith("api-ms-win-crt-") or
            n.lower() in UCRT_NAMES]
    print(f"  UCRT 动态依赖: {len(ucrt)} 个（0 = 已静态链接）")
    print()
    print(f"  判定    : {variant.upper()}  —  {toolchain}")
    print(f"  适用系统: {target}")
    if variant == "main":
        print("  提醒    : 此包在 Win7 上会 0xc0000005；Win7 请用 frameanalyzer-win7 的产物")

    if expect != "any" and variant != expect:
        print(f"\n  [FAIL] 期望 {expect}，实际 {variant} —— 包拿错了或构建配置被改动")
        return 1
    print(f"\n  结论: 校验通过 ✓ (期望 {expect})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
