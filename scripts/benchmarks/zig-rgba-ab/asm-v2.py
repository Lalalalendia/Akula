#!/usr/bin/env python3
"""GNU objdump machine instruction accounting; counts are static, not throughput."""
import json
import re
import subprocess
from collections import Counter
from pathlib import Path

dest = Path(".rgba-comparison-v2/asm")
dest.mkdir(parents=True, exist_ok=True)
spec = [
    ("rust_generic", "rgba_half_packed"),
    ("rust_generic", "rgba_half_unaligned"),
    ("rust_generic", "rgba_half_row"),
    ("rust_native", "rgba_half_packed"),
    ("rust_native", "rgba_half_unaligned"),
    ("rust_native", "rgba_half_row"),
    ("zig_generic", "rgba_half_packed"),
    ("zig_native", "rgba_half_packed"),
]
results = []
for lib, symbol in spec:
    libpath = Path(".rgba-comparison-v2") / (lib + ".so")
    dump = subprocess.check_output(["objdump", "-d", "-M", "intel",
                                    "--no-show-raw-insn",
                                    "--disassemble=" + symbol, str(libpath)],
                                   text=True)
    asm_path = dest / (lib + "_" + symbol + ".asm")
    asm_path.write_text(dump)
    ops = re.findall(r"^\s*[a-f0-9]+:\s+([a-z][a-z0-9.]*)\b(.*)$",
                     dump, re.MULTILINE)
    if not ops:
        raise RuntimeError(f"No disassembly for {lib}:{symbol}")
    counts = Counter(name for name, _ in ops)
    simd = sum(bool(re.search(r"\b(?:xmm|ymm|zmm)\d+\b", operand))
               for _, operand in ops)
    branches = sum(name.startswith("j") for name, _ in ops)
    print(f"ASM {lib}:{symbol} static instructions={len(ops)} "
          f"SIMD-register={simd} branches={branches}")
    results.append({"library": lib, "symbol": symbol,
                    "instructions": len(ops), "simd_register_instructions": simd,
                    "branches": branches,
                    "top_mnemonics": counts.most_common(24),
                    "path": str(asm_path)})
Path(".rgba-comparison-v2/assembly-summary.json").write_text(
    json.dumps(results, indent=2))
