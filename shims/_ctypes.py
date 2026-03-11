"""Minimal _ctypes substrate for pyrs scientific-stack import paths.

This is a temporary compatibility layer to unblock ctypes-dependent imports
(e.g. SciPy _ccallback). It intentionally implements only a narrow runtime
surface and should be replaced by a native Rust _ctypes substrate.
"""

from __future__ import annotations

__version__ = "1.1.0"

RTLD_LOCAL = 0
RTLD_GLOBAL = 0

FUNCFLAG_CDECL = 0x1
FUNCFLAG_PYTHONAPI = 0x2
FUNCFLAG_USE_ERRNO = 0x8
FUNCFLAG_USE_LASTERROR = 0x10

SIZEOF_TIME_T = 8

_errno_value = 0
_last_error_value = 0


def _pointer_size() -> int:
    return 8


def _long_size() -> int:
    return 4


def _ulong_size() -> int:
    return 4


def _longlong_size() -> int:
    return 8


def _wchar_size() -> int:
    # CPython's ctypes treats c_wchar as wchar_t.
    # On modern Unix/macOS this is 4 bytes.
    return 4


def _size_from_type_code(code: str) -> int:
    if code in {"P", "z", "O"}:
        return _pointer_size()
    if code == "h":
        return 2
    if code == "H":
        return 2
    if code == "i":
        return 4
    if code == "I":
        return 4
    if code == "q":
        return _longlong_size()
    if code == "Q":
        return _longlong_size()
    if code == "f":
        return 4
    if code == "d":
        return 8
    if code == "?":
        return 1
    if code == "b":
        return 1
    if code == "B":
        return 1
    if code == "c":
        return 1
    if code == "l":
        return _long_size()
    if code == "L":
        return _ulong_size()
    if code == "g":
        return 16
    if code == "u":
        return _wchar_size()
    raise AttributeError(f"unsupported ctypes type code: {code!r}")


class ArgumentError(Exception):
    pass


class CField:
    pass


class _CTypeMeta(type):
    def __mul__(cls, length):
        if not isinstance(length, int):
            return NotImplemented
        if length < 0:
            raise ValueError("Array length must be >= 0")
        return PyCArrayType(
            f"{cls.__name__}_Array_{length}",
            (Array,),
            {"_type_": cls, "_length_": length},
        )

    __rmul__ = __mul__


class PyCStructType(_CTypeMeta):
    pass


class UnionType(_CTypeMeta):
    pass


class PyCPointerType(_CTypeMeta):
    pass


class PyCArrayType(_CTypeMeta):
    pass


class PyCSimpleType(_CTypeMeta):
    pass


class PyCFuncPtrType(_CTypeMeta):
    pass


class _CData:
    _type_ = "O"

    def __init__(self, value=0):
        self.value = value

    @classmethod
    def from_buffer(cls, obj):
        value = getattr(obj, "value", 0)
        return cls(value)

    @classmethod
    def from_param(cls, value):
        if isinstance(value, cls):
            return value
        return cls(value)

    @classmethod
    def from_address(cls, _address):
        return cls()

    @classmethod
    def from_buffer_copy(cls, obj):
        return cls.from_buffer(obj)

    def __int__(self):
        try:
            return int(self.value)
        except Exception:
            return 0


class _SimpleCData(_CData, metaclass=PyCSimpleType):
    _type_ = "O"


class Union(_CData, metaclass=UnionType):
    pass


class Structure(_CData, metaclass=PyCStructType):
    pass


class Array(_CData, metaclass=PyCArrayType):
    _length_ = 0


class _Pointer(_CData, metaclass=PyCPointerType):
    _type_ = "P"


class CFuncPtr(_CData, metaclass=PyCFuncPtrType):
    _argtypes_ = ()
    _restype_ = None
    _flags_ = 0

    def __init__(self, target=None):
        super().__init__(0)
        self.argtypes = tuple(getattr(type(self), "_argtypes_", ()))
        self.restype = getattr(type(self), "_restype_", None)
        self._flags = int(getattr(type(self), "_flags_", 0))
        self._callable = None
        self._name = None
        self._dll = None

        if callable(target):
            self._callable = target
            self._address = id(target)
        elif isinstance(target, tuple):
            # (name, dll) / (ordinal, dll) form
            if target:
                self._name = target[0]
            if len(target) > 1:
                self._dll = target[1]
            self._address = id(self)
        elif target is None:
            self._address = 0
        else:
            try:
                self._address = int(target)
            except Exception:
                self._address = id(target)

    def __call__(self, *args, **kwargs):
        if self._callable is not None:
            return self._callable(*args, **kwargs)
        if self._address == _cast_addr:
            if len(args) < 3:
                raise TypeError("cast helper expects (obj, ignored, typ)")
            obj, _ignored, typ = args[:3]
            value = obj
            if isinstance(value, _CData):
                value = value.value
            try:
                return typ(value)
            except Exception:
                return typ()
        if self._address == _string_at_addr:
            return b""
        if self._address == _memoryview_at_addr:
            size = 0
            if len(args) > 1:
                try:
                    size = int(args[1])
                except Exception:
                    size = 0
            if size < 0:
                size = 0
            return memoryview(b"\x00" * size)
        if self._address == _memmove_addr or self._address == _memset_addr:
            return args[0] if args else None
        return None


# Sentinel trampoline addresses consumed by ctypes.py's CFUNCTYPE wrappers.
_memmove_addr = 0x1001
_memset_addr = 0x1002
_string_at_addr = 0x1003
_cast_addr = 0x1004
_memoryview_at_addr = 0x1005


def sizeof(typ):
    if isinstance(typ, type):
        code = getattr(typ, "_type_", None)
        if code is None:
            raise TypeError("type has no _type_ attribute")
        return _size_from_type_code(code)
    code = getattr(type(typ), "_type_", None)
    if code is None:
        raise TypeError("object type has no _type_ attribute")
    return _size_from_type_code(code)


def alignment(typ):
    # Good enough for import-time checks in ctypes.py
    return sizeof(typ)


def byref(obj):
    return obj


def addressof(obj):
    return id(obj)


def resize(_obj, _size):
    return None


def get_errno():
    return _errno_value


def set_errno(value):
    global _errno_value
    _errno_value = int(value)
    return _errno_value


def get_last_error():
    return _last_error_value


def set_last_error(value):
    global _last_error_value
    _last_error_value = int(value)
    return _last_error_value


def dlopen(_name, _mode=RTLD_LOCAL):
    # Non-zero sentinel handle for import-time construction of CDLL/PyDLL.
    return 1


def LoadLibrary(_name, _mode=0):
    return 1
