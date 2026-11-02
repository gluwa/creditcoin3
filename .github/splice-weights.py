#!/usr/bin/env python3
"""Replace one function's generated weights (doc comment and body) in a weights.rs with the
same function from another benchmark run. It appears once per impl, so every occurrence is
replaced in order.

Usage: splice-weights.py <weights.rs> <partial.rs> <function>
"""
import re
import sys

target, partial, name = sys.argv[1:4]
block = re.compile(
    r"(?:\t///[^\n]*\n)*\tfn " + re.escape(name) + r"\(([^)]*)\) -> Weight \{\n.*?\n\t\}\n",
    re.DOTALL,
)

old = open(target).read()
new_blocks = [m.group(0) for m in block.finditer(open(partial).read())]
old_count = len(block.findall(old))
if not new_blocks or len(new_blocks) != old_count:
    sys.exit(f"{name}: {old_count} in {target}, {len(new_blocks)} in {partial}")

it = iter(new_blocks)
open(target, "w").write(block.sub(lambda _: next(it), old))
