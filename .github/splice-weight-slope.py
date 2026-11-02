#!/usr/bin/env python3
"""Copy one component's slope for a function from a partial benchmark run into a weights.rs.

If the target has no slope for the component (generated as `_<component>`), it is added.

Usage: splice-weight-slope.py <weights.rs> <partial.rs> <function> <component>
"""
import re
import sys

target, partial, name, comp = sys.argv[1:5]

func = re.compile(
    r"(?P<head>(?:\t///[^\n]*\n)*\tfn " + re.escape(name) + r"\()(?P<args>[^)]*)(?P<tail>\) -> Weight \{\n.*?\n\t\}\n)",
    re.DOTALL,
)
term = re.compile(
    r"(?:\t\t\t// Standard Error: [\d_]+\n)?\t\t\t\.saturating_add\(Weight::from_parts\([\d_]+, 0\)"
    r"\.saturating_mul\(" + re.escape(comp) + r"\.into\(\)\)\)\n"
)
proof_size = re.compile(r"\t\t\t\.saturating_add\(Weight::from_parts\(0, [\d_]+\)\)\n")

new_func = func.search(open(partial).read())
new_term = term.search(new_func.group(0)) if new_func else None
if not new_term:
    sys.exit(f"{name}: no `{comp}` slope in {partial}")

old = open(target).read()
if len(func.findall(old)) != 1:
    sys.exit(f"{name}: expected exactly one in {target}")


def patch(m):
    body = m.group(0)
    if term.search(body):
        return term.sub(lambda _: new_term.group(0), body, count=1)
    body = body.replace(f"_{comp}: u32", f"{comp}: u32", 1)
    return proof_size.sub(lambda p: p.group(0) + new_term.group(0), body, count=1)


open(target, "w").write(func.sub(patch, old, count=1))
