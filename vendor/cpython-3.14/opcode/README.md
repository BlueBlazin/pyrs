# Opcode Metadata

We will vendor CPython 3.14 opcode definitions and metadata here.

Primary sources (synced from a CPython checkout):
- `opcode.py` (Lib/opcode.py)
- `bytecodes.c` (Python/bytecodes.c)
- `opcode.h` (Include/opcode.h)

Derived artifacts (generated later):
- opcode_table.csv (opcode number, name, stack effect, flags)
- magic_number.txt (pyc magic number)
- bytecode_version.txt
