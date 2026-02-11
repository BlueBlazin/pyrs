# Unicode Name Data (`\N{...}`)

`src/parser/unicode_names_data.txt` is generated data. Do not edit it by hand.

## Provenance

The generator is `scripts/generate_unicode_name_table.py`.

It combines two CPython 3.14 sources of truth:

1. Canonical character names (`C;...`) from `unicodedata.name(chr(cp))` for all assigned code points.
2. Name aliases (`A;...`) and named sequences (`S;...`) decoded from CPython's generated Unicode DB:
   - `Modules/unicodename_db.h`
   - DAWG payload + codepoint->name-position index tables
   - `name_aliases[]` and `named_sequences[]`

This mirrors CPython's own Unicode-name database structure rather than a hand-maintained list.

## Runtime Semantics

- Parser/lexer `\N{...}` accepts:
  - canonical names (`C`)
  - aliases (`A`)
- Parser/lexer `\N{...}` rejects named sequences (`S`), matching CPython literal semantics.
  - Named sequences remain present in data for explicit parity accounting and future `unicodedata` surfaces.

## Regeneration

Use CPython 3.14 source plus Python 3.14 runtime:

```bash
python3.14 scripts/generate_unicode_name_table.py --cpython-src /path/to/Python-3.14.3
```

Verification mode (no write):

```bash
python3.14 scripts/generate_unicode_name_table.py --check --cpython-src /path/to/Python-3.14.3
```

`PYRS_CPYTHON_SRC` can be used instead of passing `--cpython-src` every run.
