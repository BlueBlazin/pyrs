"""Minimal enum compatibility shim for pyrs bootstrap."""

import sys


class Enum:
    def __init__(self, *args, **kwargs):
        self._args = args
        self._kwargs = kwargs

    @property
    def name(self):
        return getattr(self, "_name_", None)

    @property
    def value(self):
        return getattr(self, "_value_", None)


class ReprEnum(Enum):
    pass


class IntEnum(int, Enum):
    pass


class StrEnum(str, Enum):
    pass


class Flag(Enum):
    pass


class IntFlag(int, Flag):
    pass


def _convert_impl(base, name, module, predicate):
    module_obj = __import__(module, globals(), locals(), [], 0)
    attrs = {}
    for key, value in module_obj.__dict__.items():
        try:
            include = predicate(key)
        except Exception:
            include = False
        if include:
            attrs[key] = value
    enum_cls = type(name, (base,), attrs)
    setattr(module_obj, name, enum_cls)
    enum_cls.__members__ = attrs
    return enum_cls


def _make_convert(base):
    def _convert_(*args):
        if len(args) == 3:
            name, module, predicate = args
        elif len(args) == 4:
            _, name, module, predicate = args
        else:
            raise TypeError("_convert_ expects name, module, predicate")
        return _convert_impl(base, name, module, predicate)

    return _convert_


Enum._convert_ = _make_convert(Enum)
IntEnum._convert_ = _make_convert(IntEnum)
StrEnum._convert_ = _make_convert(StrEnum)
Flag._convert_ = _make_convert(Flag)
IntFlag._convert_ = _make_convert(IntFlag)


class FlagBoundary:
    pass


class EnumCheck:
    CONTINUOUS = "CONTINUOUS"
    NAMED_FLAGS = "NAMED_FLAGS"
    UNIQUE = "UNIQUE"


CONTINUOUS = EnumCheck.CONTINUOUS
NAMED_FLAGS = EnumCheck.NAMED_FLAGS
UNIQUE = EnumCheck.UNIQUE

EnumType = type
EnumMeta = type


class EnumDict(dict):
    pass


class auto:
    def __init__(self):
        self.value = None


KEEP = object()
STRICT = object()
CONFORM = object()
EJECT = object()


def unique(cls):
    return cls


def global_enum(cls, update_str=False):
    module_obj = None
    try:
        frame = sys._getframe(1)
        module_name = frame.f_globals.get("__name__")
        module_obj = sys.modules.get(module_name)
    except Exception:
        module_obj = None
    if module_obj is None:
        module_name = getattr(cls, "__module__", None)
        if module_name:
            module_obj = sys.modules.get(module_name)
            if module_obj is None:
                try:
                    module_obj = __import__(module_name, globals(), locals(), [], 0)
                except Exception:
                    module_obj = None
    if module_obj is not None:
        for name, value in cls.__dict__.items():
            if isinstance(name, str) and name.startswith("_"):
                continue
            setattr(module_obj, name, value)
    return cls


def global_enum_repr(value):
    return repr(value)


def global_flag_repr(value):
    return repr(value)


def global_str(value):
    return str(value)


class verify:
    def __init__(self, *checks):
        self.checks = checks

    def __call__(self, cls):
        return cls


def member(value):
    return value


def nonmember(value):
    return value


def _iter_bits_lsb(value):
    index = 0
    current = value
    while current:
        if current & 1:
            yield 1 << index
        current >>= 1
        index += 1


def _simple_enum(etype=Enum, boundary=None, use_args=False):
    def decorator(cls):
        attrs = {}
        for key, value in cls.__dict__.items():
            if key in {"__dict__", "__weakref__"}:
                continue
            attrs[key] = value
        enum_cls = type(cls.__name__, (etype,), attrs)
        members = {}
        for name, raw_value in list(attrs.items()):
            if not isinstance(name, str) or name.startswith("_"):
                continue
            if name.islower():
                continue
            if callable(raw_value) or isinstance(raw_value, (classmethod, staticmethod, property)):
                continue
            if isinstance(raw_value, tuple):
                ctor_args = raw_value
            else:
                ctor_args = (raw_value,)
            try:
                member = enum_cls.__new__(enum_cls, *ctor_args)
            except Exception:
                member = raw_value
            try:
                setattr(member, "_name_", name)
            except Exception:
                pass
            try:
                if not hasattr(member, "_value_"):
                    setattr(member, "_value_", ctor_args[0] if ctor_args else None)
            except Exception:
                pass
            setattr(enum_cls, name, member)
            members[name] = member
        enum_cls.__members__ = members
        return enum_cls

    return decorator


def _test_simple_enum(_checked_enum, _simple_enum):
    return True
