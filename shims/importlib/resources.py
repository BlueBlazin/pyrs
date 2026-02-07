"""Local importlib.resources shim for environments without a discoverable CPython stdlib."""

import importlib.util
import os


def _package_name(package):
    if isinstance(package, str):
        return package
    return getattr(package, '__name__', package)


class _ResourcePath:
    def __init__(self, path):
        self._path = path

    def joinpath(self, name):
        return _ResourcePath(os.path.join(self._path, name))

    def __truediv__(self, name):
        return self.joinpath(name)

    def read_text(self, encoding='utf-8'):
        return open(self._path, 'r')

    def read_bytes(self):
        return open(self._path, 'rb')

    def open(self, mode='r', encoding='utf-8'):
        return open(self._path, mode)


def files(package):
    package_name = _package_name(package)
    spec = importlib.util.find_spec(package_name)
    if spec is None:
        raise ModuleNotFoundError(package_name)
    locations = spec['submodule_search_locations']
    if locations is not None and len(locations) > 0:
        return _ResourcePath(locations[0])
    origin = spec['origin']
    if origin is None:
        raise FileNotFoundError(package_name)
    return _ResourcePath(os.path.dirname(origin))


def read_text(package, resource, encoding='utf-8'):
    return files(package).joinpath(resource).read_text(encoding=encoding)


def read_binary(package, resource):
    return files(package).joinpath(resource).read_bytes()


def open_text(package, resource, encoding='utf-8'):
    return files(package).joinpath(resource).open('r', encoding=encoding)


def open_binary(package, resource):
    return files(package).joinpath(resource).open('rb')
