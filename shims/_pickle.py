"""Fallback _pickle shim for pyrs.

This provides enough accelerator surface for stdlib `pickle` tests while
delegating behavior to the pure-Python implementation.
"""

import sys


class _RawBuffer(bytes):
    contiguous = True
    readonly = True

    def tobytes(self):
        return bytes(self)

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False


class PickleBuffer:
    def __init__(self, obj):
        self._released = False
        self._view = memoryview(obj)

    def raw(self):
        if self._released:
            raise ValueError("operation forbidden on released PickleBuffer object")
        return _RawBuffer(bytes(self._view))

    def release(self):
        if self._released:
            return
        self._released = True
        try:
            self._view.release()
        except Exception:
            # Keep behavior forgiving for runtimes where memoryview.release
            # is not fully implemented.
            pass


def _pickle_module():
    module = sys.modules.get("pickle")
    if module is not None:
        return module
    import pickle as module

    return module


class PickleError(Exception):
    pass


class PicklingError(PickleError):
    pass


class UnpicklingError(PickleError):
    pass


class Pickler:
    def __init__(self, file, protocol=None, *, fix_imports=True, buffer_callback=None):
        self._impl = _pickle_module()._Pickler(
            file,
            protocol,
            fix_imports=fix_imports,
            buffer_callback=buffer_callback,
        )
        self._sync_dispatch_table()

    def _sync_dispatch_table(self):
        if "dispatch_table" in self.__dict__:
            self._impl.dispatch_table = self.__dict__["dispatch_table"]
            return
        class_dispatch = type(self).__dict__.get("dispatch_table")
        if class_dispatch is not None:
            self._impl.dispatch_table = class_dispatch

    def dump(self, obj):
        self._sync_dispatch_table()
        return self._impl.dump(obj)

    def __getattr__(self, name):
        if name == "dispatch_table":
            if "dispatch_table" in self.__dict__:
                return self.__dict__["dispatch_table"]
            class_dispatch = type(self).__dict__.get("dispatch_table")
            if class_dispatch is not None:
                return class_dispatch
        return getattr(self._impl, name)

    def __setattr__(self, name, value):
        object.__setattr__(self, name, value)
        if name == "_impl" or "_impl" not in self.__dict__:
            return
        if name == "dispatch_table":
            self._impl.dispatch_table = value


class Unpickler:
    def __init__(self, file, *, fix_imports=True, encoding="ASCII", errors="strict", buffers=None):
        self._impl = _pickle_module()._Unpickler(
            file,
            fix_imports=fix_imports,
            encoding=encoding,
            errors=errors,
            buffers=buffers,
        )

    def __getattr__(self, name):
        return getattr(self._impl, name)


def dump(obj, file, protocol=None, *, fix_imports=True, buffer_callback=None):
    return _pickle_module()._dump(
        obj,
        file,
        protocol,
        fix_imports=fix_imports,
        buffer_callback=buffer_callback,
    )


def dumps(obj, protocol=None, *, fix_imports=True, buffer_callback=None):
    return _pickle_module()._dumps(
        obj,
        protocol,
        fix_imports=fix_imports,
        buffer_callback=buffer_callback,
    )


def load(file, *, fix_imports=True, encoding="ASCII", errors="strict", buffers=None):
    return _pickle_module()._load(
        file,
        fix_imports=fix_imports,
        encoding=encoding,
        errors=errors,
        buffers=buffers,
    )


def loads(s, /, *, fix_imports=True, encoding="ASCII", errors="strict", buffers=None):
    return _pickle_module()._loads(
        s,
        fix_imports=fix_imports,
        encoding=encoding,
        errors=errors,
        buffers=buffers,
    )


__all__ = [
    "PickleBuffer",
    "PickleError",
    "PicklingError",
    "UnpicklingError",
    "Pickler",
    "Unpickler",
    "dump",
    "dumps",
    "load",
    "loads",
]
