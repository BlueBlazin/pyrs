"""Minimal pyexpat compatibility shim for ElementTree core parsing paths.

This intentionally implements only the parser surface used by stdlib XMLParser
for common in-memory parse flows (ParserCreate + Parse + callback handlers).
"""

from __future__ import annotations

import re
import types


class ExpatError(Exception):
    def __init__(self, message: str, code: int = 1, lineno: int = 1, offset: int = 0):
        super().__init__(message)
        self.code = code
        self.lineno = lineno
        self.offset = offset


error = ExpatError
version_info = (2, 6, 0)

model = types.ModuleType("pyexpat.model")
errors = types.ModuleType("pyexpat.errors")


_ATTR_RE = re.compile(r'''([^\s=/>]+)\s*=\s*(?:"([^"]*)"|'([^']*)')''')


class _Parser:
    def __init__(self, encoding=None, namespace_separator=None):
        self.encoding = encoding
        self.namespace_separator = namespace_separator
        self.buffer_text = 0
        self.ordered_attributes = 0
        self.ErrorLineNumber = 1
        self.ErrorColumnNumber = 0
        self.StartElementHandler = None
        self.EndElementHandler = None
        self.StartNamespaceDeclHandler = None
        self.EndNamespaceDeclHandler = None
        self.CharacterDataHandler = None
        self.CommentHandler = None
        self.ProcessingInstructionHandler = None
        self.DefaultHandlerExpand = None
        self._buffer: list[str] = []
        self._stack: list[str] = []
        self._reparse_deferral_enabled = True

    def _set_position(self, text: str, index: int) -> None:
        if index <= 0:
            self.ErrorLineNumber = 1
            self.ErrorColumnNumber = 0
            return
        head = text[:index]
        self.ErrorLineNumber = head.count("\n") + 1
        last_nl = head.rfind("\n")
        self.ErrorColumnNumber = index if last_nl < 0 else (index - last_nl - 1)

    def _raise_syntax(self, text: str, index: int, message: str) -> None:
        self._set_position(text, index)
        raise ExpatError(
            message,
            code=1,
            lineno=self.ErrorLineNumber,
            offset=self.ErrorColumnNumber,
        )

    def _parse_start_tag(self, segment: str) -> tuple[str, list[tuple[str, str]]]:
        if not segment:
            raise ExpatError("empty start tag")
        parts = segment.split(None, 1)
        tag = parts[0]
        if not tag:
            raise ExpatError("missing element name")
        attrs: list[tuple[str, str]] = []
        if len(parts) > 1:
            for match in _ATTR_RE.finditer(parts[1]):
                key = match.group(1)
                value = match.group(2)
                if value is None:
                    value = match.group(3) or ""
                attrs.append((key, value))
        return tag, attrs

    def _emit_start(self, tag: str, attrs: list[tuple[str, str]]) -> None:
        handler = self.StartElementHandler
        if handler is None:
            return
        if self.ordered_attributes:
            ordered: list[str] = []
            for key, value in attrs:
                ordered.append(key)
                ordered.append(value)
            handler(tag, ordered)
            return
        handler(tag, {key: value for key, value in attrs})

    def _parse_text(self, text: str) -> None:
        index = 0
        length = len(text)
        while index < length:
            ch = text[index]
            if ch != "<":
                next_lt = text.find("<", index)
                if next_lt < 0:
                    next_lt = length
                data = text[index:next_lt]
                if data and self.CharacterDataHandler is not None:
                    self.CharacterDataHandler(data)
                index = next_lt
                continue

            if text.startswith("<!--", index):
                end = text.find("-->", index + 4)
                if end < 0:
                    self._raise_syntax(text, index, "unclosed comment")
                if self.CommentHandler is not None:
                    self.CommentHandler(text[index + 4 : end])
                index = end + 3
                continue

            if text.startswith("<?", index):
                end = text.find("?>", index + 2)
                if end < 0:
                    self._raise_syntax(text, index, "unclosed processing instruction")
                if self.ProcessingInstructionHandler is not None:
                    body = text[index + 2 : end].strip()
                    if body:
                        parts = body.split(None, 1)
                        target = parts[0]
                        data = parts[1] if len(parts) > 1 else ""
                        self.ProcessingInstructionHandler(target, data)
                index = end + 2
                continue

            if text.startswith("<![CDATA[", index):
                end = text.find("]]>", index + 9)
                if end < 0:
                    self._raise_syntax(text, index, "unclosed CDATA section")
                if self.CharacterDataHandler is not None:
                    self.CharacterDataHandler(text[index + 9 : end])
                index = end + 3
                continue

            if text.startswith("<!", index):
                end = text.find(">", index + 2)
                if end < 0:
                    self._raise_syntax(text, index, "unclosed declaration")
                if self.DefaultHandlerExpand is not None:
                    self.DefaultHandlerExpand(text[index : end + 1])
                index = end + 1
                continue

            if text.startswith("</", index):
                end = text.find(">", index + 2)
                if end < 0:
                    self._raise_syntax(text, index, "unclosed end tag")
                tag = text[index + 2 : end].strip()
                if not tag:
                    self._raise_syntax(text, index, "missing end tag name")
                if self._stack:
                    expected = self._stack.pop()
                    if expected != tag:
                        self._raise_syntax(text, index, "mismatched end tag")
                if self.EndElementHandler is not None:
                    self.EndElementHandler(tag)
                index = end + 1
                continue

            end = text.find(">", index + 1)
            if end < 0:
                self._raise_syntax(text, index, "unclosed start tag")
            segment = text[index + 1 : end].strip()
            self_closing = segment.endswith("/")
            if self_closing:
                segment = segment[:-1].rstrip()
            try:
                tag, attrs = self._parse_start_tag(segment)
            except ExpatError:
                self._set_position(text, index)
                raise
            self._emit_start(tag, attrs)
            if self_closing:
                if self.EndElementHandler is not None:
                    self.EndElementHandler(tag)
            else:
                self._stack.append(tag)
            index = end + 1

        if self._stack:
            self._raise_syntax(text, len(text), "unclosed element")

    def Parse(self, data, isfinal=False):
        if isinstance(data, bytes):
            chunk = data.decode(self.encoding or "utf-8")
        elif isinstance(data, str):
            chunk = data
        else:
            raise TypeError("a bytes-like object is required")
        if chunk:
            self._buffer.append(chunk)
        if not isfinal:
            return 1
        text = "".join(self._buffer)
        self._buffer = []
        self._stack = []
        if text:
            self._parse_text(text)
        return 1

    def GetReparseDeferralEnabled(self):
        return self._reparse_deferral_enabled

    def SetReparseDeferralEnabled(self, enabled):
        self._reparse_deferral_enabled = bool(enabled)


def ParserCreate(encoding=None, namespace_separator=None):
    return _Parser(encoding=encoding, namespace_separator=namespace_separator)


__all__ = [
    "ParserCreate",
    "ExpatError",
    "error",
    "version_info",
    "model",
    "errors",
]
