"""Local pkgutil shim for environments without a discoverable CPython stdlib."""

import importlib.util
import os


def get_data(package, resource):
    spec = importlib.util.find_spec(package)
    if spec is None:
        return None
    locations = spec['submodule_search_locations']
    if locations is None or len(locations) == 0:
        return None
    path = os.path.join(locations[0], resource)
    try:
        handle = open(path, 'rb')
        try:
            return handle.read()
        finally:
            handle.close()
    except Exception:
        return None


def iter_modules(path=None, prefix=''):
    return []


def walk_packages(path=None, prefix='', onerror=None):
    return []


def resolve_name(name, package=None):
    return name
