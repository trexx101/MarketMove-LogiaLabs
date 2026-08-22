# Editing .ipynb files safely

## Strategy A — load-modify-dump (preferred, worked cleanly)

When the notebook is still valid JSON, the simplest and safest edit path
is to parse it as a Python object, modify the `source` array in-place,
and dump it back. `json.load` / `json.dump` handle all escaping
automatically — no text surgery, no comma-counting, no escaping bugs.

### Recipe (worked on MarketMoves QQQ_Equities_Model.ipynb)

```python
import json

nb = json.load(open('notebook.ipynb'))

# Replace an entire cell's source
nb['cells'][9]['source'] = [
    "def robust_normalize(train_feats, val_feats=None):\n",
    "    medians = train_feats.median()\n",
    "    mads = (train_feats - medians).abs().median()\n",
    "    for col, floor in MAD_FLOORS.items():\n",
    "        if col in mads.index:\n",
    "            mads[col] = max(mads[col], floor)\n",
    "    ...\n",
]

# Insert lines after a marker in another cell
new_src = []
for line in nb['cells'][13]['source']:
    new_src.append(line)
    if 'final_train_norm, final_stats = robust_normalize(features)' in line:
        new_src.append('\n')
        new_src.append('    _sanity_check_norm_stats(final_stats, features)\n')
        new_src.append('    print("norm_stats sanity check PASSED")\n')
nb['cells'][13]['source'] = new_src

json.dump(nb, open('notebook.ipynb', 'w'), indent=1)
```

### Why this works
- `json.load` gives you native Python lists/dicts — you edit `source` as
  a plain list of strings, each ending in `\n`.
- `json.dump` re-escapes everything correctly: quotes, backslashes,
  newlines, commas, structure. Zero chance of the corruption that
  `patch` causes.
- Preserves all metadata, cell IDs, and outputs that a full rebuild
  would discard.

### When NOT to use this
- The file is already corrupted (invalid JSON) — `json.load` will fail.
  Use the clean rebuild strategy instead.
- You need byte-exact formatting outside the edited block (rare).

---

## Strategy C — clean rebuild (when already corrupted)

### Context
During MarketMarkovNet work we edited a Colab notebook
`models/Crypto_Markov_Head.ipynb` with `patch`. The patch left
literal newlines inside a `"source": [...]` JSON string array and dropped
trailing commas. Result: `json.load` failed with
`Invalid control character at: line 63 column 71`.

Three `patch`/"repair" attempts on the now-invalid file all failed
(metadata-before-source vs after-source locator bugs, byte-offset shifts).
The fix was a clean rebuild via `write_file` of the whole notebook.

### Rebuild recipe (worked)
1. Read every code cell's source this session (we had it from prior reads).
2. `write_file` the ENTIRE `.ipynb` as valid JSON:
   - `cells` array; each cell `{cell_type, metadata:{id}, source:[...], outputs:[]}`.
   - For `"source"` arrays: each line is a JSON string ending with `",`
     EXCEPT the last (ends `"]`). Newlines WITHIN a source line are
     `\\n` in the JSON; quotes are `\"`; backslashes `\\`.
   - Drop `outputs` (rendered PNG `display_data`, stream text) — empty
     `[]` is fine; they are not needed to re-run.
3. Verify: `python3 -c "import json; json.load(open(path))"` exit 0.
4. Then assert content: extract code-cell sources, `ast.parse` each,
   and grep for the changed substrings.

### Gotchas
- In ipynb, a cell's `"id"` can sit BEFORE `"source"` (metadata-first)
  or AFTER it. A `find`/`rfind` locator keyed on `"id"` is wrong
  half the time once the file is already broken. Don't bother repairing
  corrupted JSON with text surgery — rebuild.
- The notebook was gitignored (a Colab training artifact, not
  source-controlled), so it is NOT in git. The committed record of
  what changed lives in the plan file, not the notebook.
- Keep cell `id`s stable if other tooling references them.
- If the user provides a clean original copy (e.g.
  `QQQ_Equities_Model_ORI.ipynb`), apply the fix to THAT file using
  Strategy A (load-modify-dump), not a rebuild.
