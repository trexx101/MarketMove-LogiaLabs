#!/usr/bin/env python3
# Reusable ad-hoc validator for structured/text-serialized docs.
# Usage: python3 verify_json_file.py <path> [--substr "..." ...] [--ipynb]
# Exit 0 = valid + checks pass; non-zero = fail.
# Designed for the safe-structured-edits skill: run from /tmp as
# hermes-verify-<topic>.py, print PASS/FAIL, then delete when done.
import sys, json, ast

def main():
    if len(sys.argv) < 2:
        print("usage: verify_json_file.py <path> [--substr S ...] [--ipynb]")
        return 2
    path = sys.argv[1]
    sub = [sys.argv[i+1] for i, a in enumerate(sys.argv) if a == "--substr"]
    ipynb = "--ipynb" in sys.argv
    try:
        with open(path, encoding="utf-8") as f:
            raw = f.read()
        nb = json.loads(raw)
        print(f"[OK] {path} parses as valid JSON")
    except Exception as e:
        print(f"[FAIL] JSON parse error: {e}")
        return 1
    problems = []
    if ipynb:
        cells = nb.get("cells", [])
        code = [c for c in cells if c.get("cell_type") == "code"]
        print(f"[OK] ipynb cells={len(cells)} code={len(code)}")
        for i, c in enumerate(code):
            src = "".join(c.get("source", []))
            # Strip Jupyter magics (! and %) before ast.parse — they are
            # valid in a kernel but not in plain Python, so compile() /
            # ast.parse() would false-positive on them.
            src_no_magic = "\n".join(
                ln for ln in src.split("\n")
                if not ln.lstrip().startswith(("!", "%"))
            )
            try:
                ast.parse(src_no_magic)
            except SyntaxError as e:
                problems.append(f"cell#{i} SyntaxError: {e}")
        if not problems:
            print("[OK] all code cells parse")
    all_src = json.dumps(nb)
    for s in sub:
        ok = s in all_src
        print(f"  [{'OK' if ok else 'MISS'}] contains: {s}")
        if not ok:
            problems.append(f"missing substring: {s}")
    if problems:
        print("=== RESULT: PROBLEMS ===")
        for p in problems:
            print("  -", p)
        return 1
    print("=== RESULT: ALL CHECKS PASSED ===")
    return 0

if __name__ == "__main__":
    sys.exit(main())
