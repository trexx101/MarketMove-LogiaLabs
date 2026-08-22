# .ipynb patch-tool raw-text corruption — concrete incident + fix

## What happened (2026-08-14, MarketMoves EQ_Equities_Model.ipynb)

The Hermes `patch` tool was used to insert new Python lines into a cell's
`"source": [...]` JSON array. The `new_string` contained raw Python
(e.g. `# SYMBOL is the equities ticker...\nSYMBOL = ...\n`)
**without** wrapping each line as a JSON-escaped string element
(`"# SYMBOL is...\n", "SYMBOL = ...\n",`).

The `patch` tool matched the `old_string` (raw text inside the array
source) and substituted the raw text `new_string`. It reported SUCCESS.
But the file's underlying JSON was now invalid because:

- The inserted Python lines appeared literally in the file (no
  surrounding quotes, no `\n` escape, no trailing `,`).
- `json.load()` failed with `JSONDecodeError: Expecting value: line 79
  column 9 (char 1912)`.

## The diagnostic (cheap)

```bash
python3 -c "import json; json.load(open('file.ipynb'))" 2>&1 | head -5
# → JSONDecodeError: Expecting value: line N column M
```

Then look at that line in `read_file` — broken `.ipynb` shows raw Python
between properly-escaped JSON string elements:

```
"warnings.filterwarnings(\"ignore\")\n",
"\n",
# SYMBOL is the equities ticker this notebook will train on.
# Defaults to 'QQQ' for backward compatibility; ...
SYMBOL = os.environ.get('SYMBOL', 'NVDA').upper()

# NVDA was illiquid / near-zero pre-2016 and did a 10:1 split ...
START_DATE = "2016-01-01" if SYMBOL == "NVDA" else "1999-01-01"
...raw Python continues here...
"_export_root = \"/content/drive/MyDrive/QuantData\"\n",  # ← proper JSON resumes
```

**The patch tool's success message is misleading.** The raw-text
substitution worked (the new bytes are present), but the file is no
longer valid JSON. Re-run `json.load()` to detect; the patch tool
itself doesn't validate the result.

## Why this happens

The `patch` tool does **fuzzy raw-text matching**. The matching is
context-aware enough to find `old_string` even when it's inside a JSON
string array, but the substitution writes `new_string` verbatim — no
JSON awareness, no automatic escaping, no comma insertion. If your
`new_string` is the *content* of a JSON string element rather than a
JSON-escaped representation of it, the file becomes broken.

The earlier patches in the same session (using `"...\n",` form
correctly) worked. Only the FIRST patch — where the author thought of
the new lines as "raw Python" instead of "JSON-escaped source lines" —
produced corruption. The mental model matters: in `.ipynb`, EVERY line
of source code inside a `"source": [...]` array is a JSON string with
its own escape rules.

## Fix recipe — replace the broken span with correctly escaped lines

Once the file is corrupted, the **safest** recovery is to find the
broken span (raw Python between two valid JSON string elements) and
replace it with the properly-escaped form. The fix is a SECOND patch
where `old_string` is the raw-Python corruption and `new_string` is the
correctly-escaped JSON source-array form:

```python
# old_string (the broken span — read_file shows it as raw Python)
        # SYMBOL is the equities ticker this notebook will train on.
        # Defaults to 'QQQ' for backward compatibility; override via env var when
        # launching the notebook in Colab, e.g. before main(): os.environ['SYMBOL']='NVDA'
        SYMBOL = os.environ.get('SYMBOL', 'NVDA').upper()

        # NVDA was illiquid / near-zero pre-2016 and did a 10:1 split in 2024; training
        # walk-forward folds on pre-2016 data pollutes the gate. Cap its history. QQQ and
        # other liquid large-caps keep the full 1999 history.
        START_DATE = "2016-01-01" if SYMBOL == "NVDA" else "1999-01-01"

        # Mount Google Drive (uncomment and run in Colab if you want to save/load from Drive)
        # from google.colab import drive
        # drive.mount('/content/drive')

        # EXPORT_DIR is keyed off SYMBOL so multiple tickers can train without collision.
        # Override SYMBOL via env var before this cell runs (e.g. os.environ['SYMBOL']='NVDA').

# new_string (the correctly escaped form — each line is its own JSON string element)
        "# SYMBOL is the equities ticker this notebook will train on.\n",
        "# Defaults to 'QQQ' for backward compatibility; override via env var when\n",
        "# launching the notebook in Colab, e.g. before main(): os.environ['SYMBOL']='NVDA'\n",
        "SYMBOL = os.environ.get('SYMBOL', 'NVDA').upper()\n",
        "\n",
        "# NVDA was illiquid / near-zero pre-2016 and did a 10:1 split in 2024; training\n",
        "# walk-forward folds on pre-2016 data pollutes the gate. Cap its history. QQQ and\n",
        "# other liquid large-caps keep the full 1999 history.\n",
        "START_DATE = \"2016-01-01\" if SYMBOL == \"NVDA\" else \"1999-01-01\"\n",
        "\n",
        "# Mount Google Drive (uncomment and run in Colab if you want to save/load from Drive)\n",
        "# from google.colab import drive\n",
        "# drive.mount('/content/drive')\n",
        "\n",
        "# EXPORT_DIR is keyed off SYMBOL so multiple tickers can train without collision.\n",
        "# Override SYMBOL via env var before this cell runs (e.g. os.environ['SYMBOL']='NVDA').\n",
```

Notice the trailing `,` after every JSON string element except the
last. The `\n` inside a string is the JSON-escaped form of a real
newline. Quotes inside the string are `\"`. This is exactly the form
`json.dump()` produces — so the easiest mental model is "always pretend
you're writing what `json.dump` would write."

## Verify the recovery

```bash
# 1. JSON validity
python3 -c "import json; json.load(open('file.ipynb'))" && echo OK

# 2. All code cells still compile (strip Jupyter magics first — they're
#    valid in Colab but not in py_compile)
python3 - <<'PY'
import json, py_compile, tempfile, os
nb = json.load(open('file.ipynb'))
src = "\n".join("".join(c['source']) for c in nb['cells']
             if c.get('cell_type') == 'code')
cleaned = "\n".join(l for l in src.splitlines()
                   if not l.lstrip().startswith(('!','%')))
with tempfile.NamedTemporaryFile('w', suffix='.py', delete=False) as f:
    f.write(cleaned); path = f.name
try:
    py_compile.compile(path, doraise=True); print("compile OK")
except py_compile.PyCompileError as e:
    print("SYNTAX ERROR:", e)
finally:
    os.unlink(path)
PY

# 3. Token presence — assert the inserted strings exist
python3 - <<'PY'
import json
nb = json.load(open('file.ipynb'))
joined = "\n".join("".join(c['source']) for c in nb['cells']
                  if c.get('cell_type') == 'code')
for tok in ["SYMBOL", "START_DATE", "compute_label_stats"]:
    assert tok in joined, f"MISSING: {tok}"
    print("found", tok)
PY
```

## When the patch tool reports success, ALSO check the file

Add this to your mental checklist whenever you patch an `.ipynb`:

1. Did the patch report success? (Patch returns a diff — verify the
   diff lines are what you expected.)
2. Does `json.load()` still work? (Run it. If not, recovery is
   mechanical: see the recipe above.)
3. Does the inserted Python still parse? (Strip magics, `py_compile`.)
4. Are all expected tokens present? (One-liner grep.)

Steps 1–4 should take ~5 seconds and prevent the silent-corruption
class of bug from ever shipping.

## Alternative — write the whole cell with `write_file`

If the broken span is large or hard to specify cleanly in `old_string`,
the nuclear option is to read the full cell's source from `read_file`,
then `write_file` the entire `.ipynb` with that cell replaced. Use
`json.dumps(<list>, indent=1)` to get the escaped form. This works but
loses any preserved formatting outside the edited cell; only do it
when the file is already corrupted and the surgical fix won't land in
1–2 tries.

## See also

- `SKILL.md` — Strategy A (load-modify-dump), Strategy B (surgical
  re-serialization), Strategy C (clean rebuild).
- `references/ipynb-rebuild.md` — the original ipynb-rebuild walkthrough
  using `json.load`/`json.dump`.
