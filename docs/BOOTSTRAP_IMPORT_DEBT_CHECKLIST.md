# Bootstrap Import Debt Checklist

Audit scope: `src/vm/vm_bootstrap_import.rs` (11,255 lines)  
Audit date: 2026-03-04

## Snapshot

- `install_builtin_module(...)` calls with literal module names: 90
- `install_builtin_module(...)` calls with dynamic names: 2 (`&sysconfigdata_name`, `&legacy_sysconfigdata_name`)
- explicit alias installs (`install_module_alias_from_existing`): 7
- explicit fallback installers (`install_*_fallback_module` + `_json` accelerator fallback): 8
- module-level `BuiltinFunction::NoOp` placeholder exports in audited bootstrap scope: none (remaining `NoOp` use is non-export class wiring, e.g. internal `__init__` placeholders).
- `TypingIdFunc` placeholder usage in bootstrap installs: none (residual runtime-only `TypingIdFunc` usage remains in non-bootstrap paths such as `functools.singledispatch.register` helper return).
- local shim fallback path support is present (`shims/`, currently `_ctypes`)

## Checklist Conventions

- `[ ]` open debt item
- line references are to `src/vm/vm_bootstrap_import.rs`
- priority tags:
  - `P0`: correctness/parity blockers
  - `P1`: high-impact semantic drift
  - `P2`: architecture/import-system debt

## 1) Placeholder and `NoOp` Runtime Semantics

- [x] `P0` Replace `_symtable.symtable` `NoOp` with real behavior or correct unsupported behavior (`_symtable` install at line 7892; function entry at line 7894):
  - `symtable()` now has a dedicated builtin with explicit 3-arg call contract and intentional-unavailable behavior (`NotImplementedError`) instead of silent `NoOp`.
- [x] `P0` Replace `faulthandler` `NoOp` surfaces with CPython-compatible semantics or intentionally unavailable behavior:
  - replaced silent `NoOp`/`Bool` placeholders with explicit builtins:
    - unsupported operations (`enable`, `dump_traceback`, `dump_traceback_later`, `cancel_dump_traceback_later`, `register`, `dump_c_stack`, `_read_null`, `_sigsegv`, `_sigabrt`, `_sigfpe`, `_stack_overflow`, `_fatal_error_c_thread`) now raise `RuntimeError("faulthandler is not supported on this platform")`,
    - `disable()` returns `None`, `is_enabled()` returns `False`, and `unregister(signum)` returns `False` with explicit call-shape validation.
- [x] `P0` Replace `_locale` placeholder semantics:
  - `_locale.strxfrm`, `_locale.strcoll`, and `_locale.nl_langinfo` now use dedicated builtins (no `NoOp`/generic `Str` placeholders),
  - `strxfrm` and `strcoll` enforce explicit argument shape and string-only contracts,
  - `nl_langinfo` now enforces integer key shape, supports `CODESET` resolution via runtime filesystem encoding, and raises `ValueError` for unsupported keys.
- [x] `P0` Remove placeholder behavior in `typing`:
  - generic placeholders (`Dict/List/Tuple/Bool/Print`) for core `typing` helper exports were replaced with dedicated builtins (`get_type_hints`, `get_origin`, `get_args`, `get_protocol_members`, `get_overloads`, `clear_overloads`, `is_typeddict`, `is_protocol`);
  - `cast`, `assert_type`, `reveal_type`, `assert_never`, `final`, and `override` now use dedicated runtime semantics (no shared identity placeholder path);
  - `overload` now has dedicated registry + decorator-dummy runtime semantics aligned with CPython’s `get_overloads()`/`clear_overloads()` behavior;
  - `runtime_checkable`, `no_type_check`, `dataclass_transform`, and `no_type_check_decorator` now use dedicated runtime semantics (including decorator wrapper state where required).
- [x] `P1` Remove placeholder behavior in `_typing`:
  - `_typing._idfunc` now uses dedicated builtin dispatch (`TypingInternalIdFunc`) with strict one-argument identity semantics (no shared `TypingIdFunc` placeholder surface).
- [x] `P1` Remove placeholder behavior in `functools`:
  - `total_ordering` now uses dedicated decorator + synthesized comparator builtins (CPython-shaped `ValueError` when no root order operation exists; no `TypingIdFunc` placeholder).
- [x] `P1` Replace `_osx_support.customize_config_vars` placeholder:
  - now wired to dedicated builtin (`OsxSupportCustomizeConfigVars`) with explicit CPython-shaped call contract (`1` positional arg, no kwargs) and mapping identity return semantics.
- [x] `P1` Replace `platform` placeholder exports:
  - bootstrap exports now use dedicated native platform builtins (`system`, `release`, `version`, `machine`, `processor`, `node`, `platform`, `python_version`, `python_implementation`, `uname`) instead of generic placeholders,
  - `uname` now returns a 6-field tuple `(system, node, release, version, machine, processor)` with string entries (CPython-shaped surface).
- [x] `P1` Replace `_opcode` placeholder lists:
  - `get_intrinsic1_descs`, `get_intrinsic2_descs`, `get_special_method_names`, and `get_nb_ops` now use dedicated opcode builtins returning CPython-shaped list payloads (module install at line 5538).
- [x] `P1` Replace `subprocess._args_from_interpreter_flags` placeholder (`List`) with CPython-shaped behavior (module install at line 7351):
  - wired to dedicated native builtin (`SubprocessArgsFromInterpreterFlags`) that mirrors CPython 3.14 flag/warnoption/xoption projection behavior.
- [x] `P1` Replace `resource.getrlimit` placeholder (`Range`) with CPython-compatible return semantics (module install at line 7256):
  - wired `resource.getrlimit` to dedicated native builtin (`ResourceGetRLimit`) backed by host `getrlimit(2)`,
  - now returns CPython-shaped `(soft, hard)` tuple and raises `ValueError("invalid resource specified")` for invalid resource ids,
  - `RLIMIT_STACK`/`RLIM_INFINITY` bootstrap constants now come from host platform values instead of static placeholders.
- [ ] `P1` Replace `weakref` bootstrap placeholders:
  - `_weakref.ref` / `weakref.ref` now export a dedicated subclassable reference type (`ReferenceType`) with explicit `__new__` / `__call__` / comparison/hash method surface;
  - `PyObject_ClearWeakRefs` lifecycle now preserves weakref-object identity while transitioning refs to dead state for `PyWeakref_GetObject` / `PyWeakref_GetRef` parity;
  - `weakref` is now included in pure-stdlib preference unloading, so CPython `Lib/weakref.py` wins when available on module path;
  - `_weakref.ref` now accepts builtin type objects represented as runtime `Value::Builtin(...)` type-constructors (for example `int`), matching CPython weakrefability needed by `functools.singledispatch` dispatch-cache paths;
  - `ReferenceType.__init__` now uses dedicated weakref init semantics (`WeakRefRefInit`) instead of generic `NoOp` bootstrap placeholder wiring;
  - builtin `dict` now exports native `popitem()` semantics, unblocking CPython `weakref.WeakKeyDictionary` internals when pure `Lib/weakref.py` is active;
  - bootstrap fallback now exports dedicated `WeakKeyDictionary`/`WeakValueDictionary` classes (no direct `Dict` aliasing); remaining debt is semantic fidelity (fallback classes are still strong-reference-backed and do not yet implement CPython weak-entry lifecycle semantics).
- [x] `P1` Replace `_weakrefset.WeakSet` placeholder mapped to builtin `Set` (module install at line 5624):
  - `_weakrefset.WeakSet` now materializes a dedicated class with explicit method surface (`__init__`, `__len__`, `__contains__`, `__iter__`, `add`, `discard`, `remove`, `clear`, `update`, `copy`),
  - `threading._dangling` now initializes as a `WeakSet` instance rather than a raw builtin set.

## 2) Explicit Stub Modules

- [x] `P0` Replace `_struct` bootstrap stub with CPython-compatible substrate or proper fallback strategy:
  - `_struct` module exports are backed by dedicated native `calcsize`/`pack`/`unpack`/`iter_unpack`/`pack_into`/`unpack_from` substrate,
  - removed explicit `"pyrs _struct stub"` marker docstring in favor of CPython-shaped module description text.
- [x] `P1` Replace `_posixsubprocess` bootstrap stub with CPython-compatible substrate:
  - `fork_exec` is backed by native subprocess spawn substrate (`builtin_posixsubprocess_fork_exec`) with explicit unix/non-unix behavior,
  - removed explicit `"pyrs _posixsubprocess stub"` marker docstring in favor of CPython-shaped module description text.
- [ ] `P2` Decide final policy for `_testsinglephase` and `_testmultiphase` test modules in production bootstrap:
  - currently explicit stubs at lines 7383 and 7391.

## 3) Builtin-First Shadowing of CPython Pure-stdlib Modules

These modules are bootstrapped as builtins even though pure stdlib modules exist in CPython `Lib/`. This creates shadowing risk and divergence unless explicitly unloaded/replaced.

### 3.1 Not currently in `PURE_STDLIB_*` unload preference groups

- [x] `P1` `platform` (line 912):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_PLATFORM_MODULES`) so CPython `Lib/platform.py` is preferred when available;
  - regex fallback parser now supports class escapes inside character classes (`\w/\W/\d/\D/\s/\S`), including CPython `platform._sys_version` pattern fragments like `[\w.+]`;
  - `sys.version` now follows CPython parser shape (`version (buildno, builddate, buildtime) [compiler]`) so pure `platform.py` can parse interpreter metadata without bootstrap fallbacks;
  - covered by `tests/vm.rs::platform_import_prefers_cpython_pure_module_when_lib_path_is_added` and `tests/vm.rs::re_platform_sys_version_parser_pattern_matches_cpython_shape`.
- [x] `P1` `os` (line 1014):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_OS_MODULES`) so CPython `Lib/os.py` is preferred when available;
  - incremental substrate parity landed to support this closure:
    - native `os.getuid` / `posix.getuid` on unix hosts;
    - native `os.readlink` / `posix.readlink` with CPython-shaped return-type behavior (`str` for str-path input, `bytes` for bytes-path input);
    - `posix` now exports open/access/seek constants (`O_RDONLY`, `O_DIRECTORY`, `F_OK`, `SEEK_*`, etc.) required by pure `Lib/os.py` consumers such as `glob`/`pathlib`.
  - covered by `tests/vm.rs::os_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `_osx_support` (line 1536):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_OSX_SUPPORT_MODULES`) so CPython `Lib/_osx_support.py` is preferred when available;
  - covered by `tests/vm.rs::osx_support_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `ssl` (line 1904):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_SSL_MODULES`) so CPython `Lib/ssl.py` is preferred when available;
  - covered by `tests/vm.rs::ssl_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `codecs` (line 2605):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_CODECS_MODULES`) so CPython `Lib/codecs.py` is preferred when available;
  - `_codecs` substrate now exports `register_error` and `lookup_error`, with built-in handler registry entries for `strict`, `ignore`, `replace`, `xmlcharrefreplace`, `backslashreplace`, and `namereplace`;
  - registry lookups now remain anchored on `_codecs` so pure `codecs.py` (`from _codecs import *`) resolves built-in handlers even after `sys.modules['codecs']` is replaced by filesystem module import;
  - covered by `tests/vm.rs::codecs_import_prefers_cpython_pure_module_when_lib_path_is_added` and `tests/vm.rs::codecs_error_handler_registry_supports_builtin_and_custom_lookup`.
- [x] `P1` `operator` (line 3439):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_OPERATOR_MODULES`) so CPython `Lib/operator.py` is preferred when available;
  - covered by `tests/vm.rs::operator_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `_colorize` (line 3675):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_COLORIZE_MODULES`) so CPython `Lib/_colorize.py` is preferred when available;
  - root-cause prerequisite fixed: CPython `type.__new__` empty-bases parity now injects `object` (matching `Objects/typeobject.c:type_new_get_bases`), restoring `collections.abc.*` MRO tails and unblocking `_colorize.py` paths that rely on `super().__setattr__` from mapping ABC mixins;
  - covered by `tests/vm.rs::colorize_import_prefers_cpython_pure_module_when_lib_path_is_added`, `tests/vm.rs::metaclass_type_new_empty_bases_include_object_in_mro`, and `tests/vm.rs::super_setattr_resolves_through_collections_abc_object_tail`.
- [x] `P1` `functools` (line 3722):
  - added to `PURE_STDLIB_FUNCTOOLS_MODULES` unload preference group so `Lib/functools.py` is preferred when available;
  - covered by `tests/vm.rs::functools_import_prefers_cpython_pure_module_when_lib_path_is_added`;
  - `_find_impl` parity follow-up on `_c3_mro` pyc execution is closed:
    translated `LOAD_FAST_AND_CLEAR`/`STORE_FAST` cellvar-slot semantics now match CPython locals-plus behavior;
    covered by `tests/vm.rs::pyc_load_fast_and_clear_cellvar_roundtrip_regression`.
- [x] `P1` `__future__` (line 4697):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_FUTURE_MODULES`) so CPython `Lib/__future__.py` is preferred when available;
  - covered by `tests/vm.rs::future_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `inspect` (line 5860):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_INSPECT_MODULES`) so CPython `Lib/inspect.py` is preferred when available;
  - restored CPython-shaped `type.__dict__` getset-descriptor contract for `__mro__` and `__dict__` (including `getset_descriptor.__get__`), unblocking pure `inspect` initialization paths;
  - covered by `tests/vm.rs::inspect_import_prefers_cpython_pure_module_when_lib_path_is_added` and `tests/vm.rs::type_getset_descriptors_expose_mro_and_dict_contract`.
- [x] `P1` `io` (line 5923):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_IO_MODULES`) so CPython `Lib/io.py` is preferred when available;
  - `_io` bootstrap export surface now includes `_io.text_encoding`, matching CPython `io.py` import requirements (`from _io import ... text_encoding ...`);
  - covered by `tests/vm.rs::io_import_prefers_cpython_pure_module_when_lib_path_is_added` and `tests/vm.rs::_io_module_exports_text_encoding_helper`.
- [x] `P1` `subprocess` (line 7351):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_SUBPROCESS_MODULES`) so CPython `Lib/subprocess.py` is preferred when available;
  - covered by `tests/vm.rs::subprocess_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `uuid` (line 7637):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_UUID_MODULES`) so CPython `Lib/uuid.py` is preferred when available;
  - covered by `tests/vm.rs::uuid_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [ ] `P1` `asyncio` (line 7659)
- [ ] `P1` `threading` (line 7846)
- [x] `P1` `signal` (line 7877):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_SIGNAL_MODULES`) so CPython `Lib/signal.py` is preferred when available;
  - covered by `tests/vm.rs::signal_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `abc` (line 8691):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_ABC_MODULES`) so CPython `Lib/abc.py` is preferred when available;
  - covered by `tests/vm.rs::abc_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `sysconfig` (line 8725):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_SYSCONFIG_MODULES`) so CPython `Lib/sysconfig/__init__.py` is preferred when available;
  - `_sysconfig` module preference now aliases filesystem presence to `sysconfig`, so stale bootstrap alias modules are unloaded alongside `sysconfig`;
  - covered by `tests/vm.rs::sysconfig_import_prefers_cpython_pure_module_when_lib_path_is_added`.
- [x] `P1` `socket` (line 8768):
  - added to pure-stdlib unload preference group (`PURE_STDLIB_SOCKET_MODULES`) so CPython `Lib/socket.py` is preferred when available;
  - covered by `tests/vm.rs::socket_import_prefers_cpython_pure_module_when_lib_path_is_added`.

### 3.2 Already in unload preference groups, but still bootstrap-first

- [ ] `P2` Validate/bootstrap-minimize for `decimal` (line 848)
- [ ] `P2` Validate/bootstrap-minimize for `json` (line 1561)
- [ ] `P2` Validate/bootstrap-minimize for `re` (line 3395)
- [ ] `P2` Validate/bootstrap-minimize for `collections` (line 4045)
- [ ] `P2` Validate/bootstrap-minimize for `types` (line 4485)
- [ ] `P2` Validate/bootstrap-minimize for `typing` (line 3765)
- [ ] `P2` Validate/bootstrap-minimize for `pathlib` (line 1499)

## 4) Alias-Copy Module Debt

Current alias mechanism allocates a new module object and copies globals from source (`install_module_alias_from_existing`, line 8586). This can drift from true module identity/behavior expectations.

- [ ] `P1` `_codecs <- codecs` (line 2637)
- [ ] `P1` `_functools <- functools` (line 3758)
- [ ] `P1` `_collections <- collections` (line 4065)
- [ ] `P1` `_signal <- signal` (line 7891)
- [ ] `P1` `_sysconfig <- sysconfig` (line 8733)
- [ ] `P1` `datetime <- _datetime` (line 8780)
- [ ] `P1` `_types <- types` (line 9022 via fallback)

## 5) Import Fallback Dispatcher Debt

Fallback dispatcher (`install_builtin_import_fallback`, line 8994) installs/aliases modules on-demand for select names.

- [ ] `P1` Audit and reduce fallback-only module materialization for:
  - `abc`, `sysconfig`, `_sysconfig`, `socket`, `datetime`, `atexit`, `_types`, `_warnings`, `_json`, `_queue`.
- [ ] `P1` Ensure each fallback path has explicit CPython equivalence tests and removal criteria.
- [ ] `P2` Consolidate fallback policy so CPython `Lib/*.py` is primary where available, with native substrate only where CPython has C substrate.

## 6) Local Shim Fallback Debt

Local shim behavior:
- shim detection (`has_local_shim_module`, line 8391)
- shim root resolution (`local_shim_root`, line 8489)
- shim source fallback in module discovery (`preferred_local_shim_source`, line 9299)

- [ ] `P1` Re-validate `_ctypes` shim policy against production goal (full interpreter parity, not long-term shim replacement).
- [ ] `P1` Keep shim fallback strictly bounded and documented; ensure no silent expansion beyond allowlisted modules.

## 7) Importlib Fallback Loader Debt

- [ ] `P1` Remove `make_file_finder_importer_fallback` synthetic importer debt:
  - fallback `FileFinder.invalidate_caches` now uses dedicated importlib finder invalidation builtin (no `NoOp` placeholder),
  - remaining debt: synthetic fallback class identity/behavior still diverges from canonical importlib `FileFinder` construction.
- [ ] `P1` Remove `fallback_loader_spec_value` synthetic loader instance debt (line 10123); prefer canonical importlib loader classes where available.
- [ ] `P2` Verify fallback loader metadata parity (`__spec__`, loader identity/class, cache behavior) across source/bytecode/namespace/extension imports.

## 8) Bootstrap Surface Inventory Debt (Tracking Completeness)

- [ ] `P2` Audit dynamic sysconfig module installs for long-term plan:
  - `&sysconfigdata_name` (line 1460)
  - `&legacy_sysconfigdata_name` (line 1469)
- [ ] `P2` Document which bootstrap-installed modules are intentional native substrates vs temporary parity bridges.
- [ ] `P2` Add CI guard to fail on new bootstrap placeholders (`NoOp`, `TypingIdFunc`, obvious generic placeholders) without explicit accounting entry.

## 9) Investigation Trigger: `itertools` Parity Debt

This investigation was triggered by `itertools` behavior and must remain explicitly tracked here.

- [x] `P0` `itertools.chain` currently returns an eager `list`, not a CPython `itertools.chain` iterator object:
  - bootstrap export wiring: `itertools` install at lines 3693-3719
  - runtime implementation: `builtin_itertools_chain` at `src/vm/builtins_collections.rs:1005`
- [x] `P0` `itertools.chain.from_iterable` currently returns an eager `list`, not lazy iterator behavior:
  - runtime implementation: `builtin_itertools_chain_from_iterable` at `src/vm/builtins_collections.rs:1022`
- [x] `P1` Converted these helpers from eager snapshots to lazy iterator objects with CPython-shaped constructor behavior:
  - `accumulate`, `combinations`, `combinations_with_replacement`, `compress`, `dropwhile`, `filterfalse`, `groupby`, `islice`, `pairwise`, `permutations`, `product`, `repeat`, `starmap`, `takewhile`, `zip_longest`, `tee`, `batched`
  - implementation entrypoints: `src/vm/builtins_collections.rs` (`builtin_itertools_*` methods)
- [x] `P1` Audited exported `itertools` callables (lines 3696-3718) for iterator/laziness/type/repr parity and converted eager helpers to iterator objects with CPython-shaped repr/type behavior where applicable.
- [x] `P1` Added differential + vm tests for iterator object identity/repr/type and lazy consumption semantics across `itertools` helper surfaces (including partial-consumption `groupby` behavior).

## Recommended Fix Order

1. Close all `NoOp` and placeholder semantics in section 1 (`P0` first).  
2. Remove/replace explicit stubs in section 2.  
3. Resolve alias-copy identity drift in section 4.  
4. Trim builtin-first pure-stdlib shadowing (section 3), module family by module family.  
5. Harden import/finder fallback correctness (sections 5 and 7).  
6. Reconfirm shim and inventory policy (sections 6 and 8) with CI enforcement.
