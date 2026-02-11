#!/usr/bin/env python3
r"""Generate parser Unicode-name lookup data for \N{...} escapes.

Output format (one entry per line):
  C;<NAME>;<HEX_CODEPOINT>        canonical Unicode character names
  A;<ALIAS_NAME>;<HEX_CODEPOINT>  NameAliases entries
  S;<SEQUENCE_NAME>;<HEX...>      NamedSequences entries

The lexer accepts C/A names and intentionally rejects S names, matching CPython
string-literal semantics (named sequences are available via unicodedata.lookup
but not via \N escapes in source literals).
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import unicodedata
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class UnicodeDbHeader:
    packed_name_dawg: list[int]
    codepoint_to_pos_index1: list[int]
    codepoint_to_pos_index2: list[int]
    codepoint_shift: int
    codepoint_notfound: int
    aliases_start: int
    aliases_end: int
    name_aliases: list[int]
    named_sequences_start: int
    named_sequences_end: int
    named_sequences: list[tuple[int, ...]]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cpython-src",
        type=Path,
        default=None,
        help="Path to CPython 3.14 source root (expects Modules/unicodename_db.h)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("src/parser/unicode_names_data.txt"),
        help="Output file path",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Fail if generated content differs from --output",
    )
    return parser.parse_args()


def _parse_int(token: str) -> int:
    return int(token, 16) if token.lower().startswith("0x") else int(token)


def _extract_c_block(text: str, symbol: str) -> str:
    marker = rf"static const [^\n]+ {re.escape(symbol)}\[\] = \{{"
    match = re.search(marker, text)
    if not match:
        raise ValueError(f"unable to locate C array '{symbol}'")
    start = match.end()
    end = text.find("};", start)
    if end < 0:
        raise ValueError(f"unterminated C array '{symbol}'")
    return text[start:end]


def _extract_int_array(text: str, symbol: str) -> list[int]:
    block = _extract_c_block(text, symbol)
    values = re.findall(r"0x[0-9A-Fa-f]+|\d+", block)
    return [_parse_int(value) for value in values]


def _extract_define(text: str, symbol: str) -> int:
    match = re.search(rf"#define\s+{re.escape(symbol)}\s+(0x[0-9A-Fa-f]+|\d+)", text)
    if not match:
        raise ValueError(f"unable to locate #define '{symbol}'")
    return _parse_int(match.group(1))


def _extract_uint_const(text: str, symbol: str) -> int:
    match = re.search(
        rf"static const unsigned int\s+{re.escape(symbol)}\s*=\s*(0x[0-9A-Fa-f]+|\d+)\s*;",
        text,
    )
    if not match:
        raise ValueError(f"unable to locate unsigned int constant '{symbol}'")
    return _parse_int(match.group(1))


def _extract_named_sequences(text: str) -> list[tuple[int, ...]]:
    block = _extract_c_block(text, "named_sequences")
    entries: list[tuple[int, ...]] = []
    for match in re.finditer(r"\{\s*(\d+)\s*,\s*\{([^}]*)\}\s*\}", block):
        declared_len = int(match.group(1))
        values = [_parse_int(token) for token in re.findall(r"0x[0-9A-Fa-f]+|\d+", match.group(2))]
        if declared_len < 0 or declared_len > len(values):
            raise ValueError(f"invalid named sequence length: {declared_len}")
        entries.append(tuple(values[:declared_len]))
    return entries


def load_unicode_db_header(cpython_src: Path) -> UnicodeDbHeader:
    header_path = cpython_src / "Modules" / "unicodename_db.h"
    if not header_path.is_file():
        raise FileNotFoundError(f"missing CPython unicode header: {header_path}")

    text = header_path.read_text(encoding="utf-8")

    return UnicodeDbHeader(
        packed_name_dawg=_extract_int_array(text, "packed_name_dawg"),
        codepoint_to_pos_index1=_extract_int_array(text, "dawg_codepoint_to_pos_index1"),
        codepoint_to_pos_index2=_extract_int_array(text, "dawg_codepoint_to_pos_index2"),
        codepoint_shift=_extract_define(text, "DAWG_CODEPOINT_TO_POS_SHIFT"),
        codepoint_notfound=_extract_define(text, "DAWG_CODEPOINT_TO_POS_NOTFOUND"),
        aliases_start=_extract_uint_const(text, "aliases_start"),
        aliases_end=_extract_uint_const(text, "aliases_end"),
        name_aliases=_extract_int_array(text, "name_aliases"),
        named_sequences_start=_extract_uint_const(text, "named_sequences_start"),
        named_sequences_end=_extract_uint_const(text, "named_sequences_end"),
        named_sequences=_extract_named_sequences(text),
    )


def decode_varint_unsigned(data: list[int], index: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        byte = data[index]
        result |= (byte & 0b0111_1111) << shift
        index += 1
        shift += 7
        if (byte & 0b1000_0000) == 0:
            return result, index


def decode_node(packed: list[int], node_offset: int) -> tuple[int, int, int]:
    x, next_offset = decode_varint_unsigned(packed, node_offset)
    node_count = x >> 1
    final = x & 1
    return node_count, final, next_offset


def decode_edge(
    packed: list[int],
    edge_index: int,
    prev_child_offset: int,
    offset: int,
) -> tuple[int, int, int, int]:
    x, next_offset = decode_varint_unsigned(packed, offset)
    if x == 0 and edge_index == 0:
        raise KeyError("decoded past final node")

    child_offset_difference = x >> 2
    len_is_one = (x >> 1) & 1
    last_edge = x & 1
    child_offset = prev_child_offset + child_offset_difference
    if len_is_one:
        size = 1
    else:
        size, next_offset = decode_varint_unsigned(packed, next_offset)
    return child_offset, last_edge, size, next_offset


def inverse_lookup_name(packed: list[int], pos: int) -> str:
    out = bytearray()
    node_offset = 0
    while True:
        _, final, edge_offset = decode_node(packed, node_offset)
        if final:
            if pos == 0:
                return out.decode("ascii")
            pos -= 1

        prev_child_offset = edge_offset
        edge_index = 0
        while True:
            child_offset, last_edge, size, edge_label_offset = decode_edge(
                packed,
                edge_index,
                prev_child_offset,
                edge_offset,
            )
            edge_index += 1
            prev_child_offset = child_offset
            descendant_count, _, _ = decode_node(packed, child_offset)
            next_pos = pos - descendant_count
            if next_pos < 0:
                out.extend(packed[edge_label_offset : edge_label_offset + size])
                node_offset = child_offset
                break
            if not last_edge:
                pos = next_pos
                edge_offset = edge_label_offset + size
                continue
            raise KeyError("unable to inverse-lookup DAWG entry")


def dawg_pos_for_codepoint(db: UnicodeDbHeader, codepoint: int) -> int:
    shift = db.codepoint_shift
    index1_offset = db.codepoint_to_pos_index1[codepoint >> shift]
    index2_pos = (index1_offset << shift) + (codepoint & ((1 << shift) - 1))
    return db.codepoint_to_pos_index2[index2_pos]


def collect_aliases(db: UnicodeDbHeader) -> dict[str, int]:
    expected = db.aliases_end - db.aliases_start
    if expected != len(db.name_aliases):
        raise ValueError(
            f"name_aliases length mismatch: expected {expected}, got {len(db.name_aliases)}"
        )

    aliases: dict[str, int] = {}
    for pseudo_codepoint in range(db.aliases_start, db.aliases_end):
        pos = dawg_pos_for_codepoint(db, pseudo_codepoint)
        if pos == db.codepoint_notfound:
            raise ValueError(f"missing DAWG position for alias codepoint U+{pseudo_codepoint:06X}")
        name = inverse_lookup_name(db.packed_name_dawg, pos)
        aliases[name] = db.name_aliases[pseudo_codepoint - db.aliases_start]
    return aliases


def collect_named_sequences(db: UnicodeDbHeader) -> dict[str, tuple[int, ...]]:
    expected = db.named_sequences_end - db.named_sequences_start
    if expected != len(db.named_sequences):
        raise ValueError(
            f"named_sequences length mismatch: expected {expected}, got {len(db.named_sequences)}"
        )

    sequences: dict[str, tuple[int, ...]] = {}
    for pseudo_codepoint in range(db.named_sequences_start, db.named_sequences_end):
        pos = dawg_pos_for_codepoint(db, pseudo_codepoint)
        if pos == db.codepoint_notfound:
            raise ValueError(
                f"missing DAWG position for named sequence codepoint U+{pseudo_codepoint:06X}"
            )
        name = inverse_lookup_name(db.packed_name_dawg, pos)
        sequences[name] = db.named_sequences[pseudo_codepoint - db.named_sequences_start]
    return sequences


def collect_canonical_names() -> dict[str, int]:
    names: dict[str, int] = {}
    for codepoint in range(sys.maxunicode + 1):
        name = unicodedata.name(chr(codepoint), None)
        if name is None:
            continue
        names[name] = codepoint
    return names


def render_output(
    canonical: dict[str, int],
    aliases: dict[str, int],
    sequences: dict[str, tuple[int, ...]],
    cpython_src: Path,
) -> str:
    lines: list[str] = []
    lines.append("# Generated by scripts/generate_unicode_name_table.py")
    lines.append(f"# Python: {sys.version.split()[0]}")
    lines.append(f"# Unicode: {unicodedata.unidata_version}")
    lines.append(f"# CPython source: {cpython_src}")

    for name in sorted(canonical):
        lines.append(f"C;{name};{canonical[name]:X}")

    for name in sorted(aliases):
        lines.append(f"A;{name};{aliases[name]:X}")

    for name in sorted(sequences):
        payload = " ".join(f"{codepoint:X}" for codepoint in sequences[name])
        lines.append(f"S;{name};{payload}")

    return "\n".join(lines) + "\n"


def main() -> int:
    args = parse_args()

    cpython_src = args.cpython_src
    if cpython_src is None:
        env_value = os.environ.get("PYRS_CPYTHON_SRC")
        if env_value:
            cpython_src = Path(env_value)

    if cpython_src is None:
        print(
            "error: missing --cpython-src (or PYRS_CPYTHON_SRC)",
            file=sys.stderr,
        )
        return 2

    db = load_unicode_db_header(cpython_src)
    canonical = collect_canonical_names()
    aliases = collect_aliases(db)
    sequences = collect_named_sequences(db)
    generated = render_output(canonical, aliases, sequences, cpython_src)

    output = args.output
    if args.check:
        existing = output.read_text(encoding="utf-8") if output.exists() else ""
        if existing != generated:
            print(f"unicode name table out of date: {output}", file=sys.stderr)
            print(
                "run: /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 "
                "scripts/generate_unicode_name_table.py "
                f"--cpython-src {cpython_src}",
                file=sys.stderr,
            )
            return 1
        print(f"unicode name table is up to date: {output}")
        return 0

    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(generated, encoding="utf-8")
    print(f"wrote {output} ({len(generated.splitlines())} lines)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
