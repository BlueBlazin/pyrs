# CPython 3.14 Vendor Snapshot

This directory holds vendored artifacts from CPython 3.14 used for compatibility.

- grammar/: Grammar reference for packrat parsing.
- opcode/: Opcode tables and bytecode metadata.

Sync from a local CPython checkout with:

```bash
python3 scripts/sync_cpython.py /path/to/cpython --version 3.14.x
```

These files will be updated when we sync to a newer CPython 3.14 release.
