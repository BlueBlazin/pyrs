"""Fallback _json accelerator shim for pyrs.

This keeps the public accelerator surface available even when the real C
extension is absent, while delegating behavior to json's pure-Python paths.
"""

import re

INFINITY = float("inf")

ESCAPE = re.compile(r'[\x00-\x1f\\"\b\f\n\r\t]')
ESCAPE_ASCII = re.compile(r'([\\"]|[^\ -~])')
ESCAPE_DCT = {
    "\\": "\\\\",
    '"': '\\"',
    "\b": "\\b",
    "\f": "\\f",
    "\n": "\\n",
    "\r": "\\r",
    "\t": "\\t",
}
for i in range(0x20):
    ESCAPE_DCT.setdefault(chr(i), "\\u{0:04x}".format(i))
del i


def encode_basestring(s):
    def replace(match):
        return ESCAPE_DCT[match.group(0)]

    return '"' + ESCAPE.sub(replace, s) + '"'


def encode_basestring_ascii(s):
    def replace(match):
        value = match.group(0)
        try:
            return ESCAPE_DCT[value]
        except KeyError:
            n = ord(value)
            if n < 0x10000:
                return "\\u{0:04x}".format(n)
            n -= 0x10000
            s1 = 0xD800 | ((n >> 10) & 0x3FF)
            s2 = 0xDC00 | (n & 0x3FF)
            return "\\u{0:04x}\\u{1:04x}".format(s1, s2)

    return '"' + ESCAPE_ASCII.sub(replace, s) + '"'


def scanstring(text, end, strict=True):
    from json.decoder import py_scanstring

    return py_scanstring(text, end, strict)


def make_scanner(context):
    from json.scanner import py_make_scanner

    return py_make_scanner(context)


def make_encoder(
    markers,
    default,
    encoder,
    indent,
    key_separator,
    item_separator,
    sort_keys,
    skipkeys,
    allow_nan,
):
    from json.encoder import _make_iterencode

    def floatstr(
        value,
        allow_nan=allow_nan,
        _repr=float.__repr__,
        _inf=INFINITY,
        _neginf=-INFINITY,
    ):
        if value != value:
            text = "NaN"
        elif value == _inf:
            text = "Infinity"
        elif value == _neginf:
            text = "-Infinity"
        else:
            return _repr(value)
        if not allow_nan:
            raise ValueError(
                "Out of range float values are not JSON compliant: " + repr(value)
            )
        return text

    return _make_iterencode(
        markers,
        default,
        encoder,
        indent,
        floatstr,
        key_separator,
        item_separator,
        sort_keys,
        skipkeys,
        True,
    )
