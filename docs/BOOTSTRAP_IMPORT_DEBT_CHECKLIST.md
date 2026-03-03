# Bootstrap Import Debt Checklist

Audit scope: `src/vm/vm_bootstrap_import.rs` (11,255 lines)  
Audit date: 2026-03-04

## Snapshot

- `install_builtin_module(...)` calls with literal module names: 90
- `install_builtin_module(...)` calls with dynamic names: 2 (`&sysconfigdata_name`, `&legacy_sysconfigdata_name`)
- explicit alias installs (`install_module_alias_from_existing`): 7
- explicit fallback installers (`install_*_fallback_module` + `_json` accelerator fallback): 8
- direct `BuiltinFunction::NoOp` usage in bootstrap installs: `_symtable`, `faulthandler`, `_locale`
- `TypingIdFunc` placeholder usage in bootstrap installs: `_osx_support`, `functools`, `typing`, `_typing`
- local shim fallback path support is present (`shims/`, currently `_ctypes`)

## Checklist Conventions

- `[ ]` open debt item
- line references are to `src/vm/vm_bootstrap_import.rs`
- priority tags:
  - `P0`: correctness/parity blockers
  - `P1`: high-impact semantic drift
  - `P2`: architecture/import-system debt

## 1) Placeholder and `NoOp` Runtime Semantics

- [ ] `P0` Replace `_symtable.symtable` `NoOp` with real behavior or correct unsupported behavior (`_symtable` install at line 7892; function entry at line 7894).
- [ ] `P0` Replace `faulthandler` `NoOp` surfaces with CPython-compatible semantics or intentionally unavailable behavior:
  - `enable`, `dump_traceback`, `dump_traceback_later`, `cancel_dump_traceback_later`, `register`, `dump_c_stack`, `_read_null`, `_sigsegv`, `_sigabrt`, `_sigfpe`, `_stack_overflow`, `_fatal_error_c_thread` (module install at line 7952).
- [ ] `P0` Replace `_locale` placeholder semantics:
  - `strcoll` and `nl_langinfo` are `NoOp`;
  - `strxfrm` is currently plain `Str` passthrough (module install at line 7983).
- [ ] `P0` Remove placeholder behavior in `typing`:
  - `TypingIdFunc` and generic placeholders (`Dict/List/Tuple/Bool/Print`) in exported API (module install at line 3765, placeholder class wiring around lines 3759-3843).
- [ ] `P1` Remove placeholder behavior in `_typing`:
  - `_idfunc` uses `TypingIdFunc` (module install at line 4910).
- [ ] `P1` Remove placeholder behavior in `functools`:
  - `total_ordering` currently wired via `TypingIdFunc` (module install at line 3722).
- [ ] `P1` Replace `_osx_support.customize_config_vars` placeholder:
  - currently wired to `TypingIdFunc` (module install at line 1536).
- [ ] `P1` Replace `platform` placeholder exports:
  - multiple functions currently mapped to `SysGetFilesystemEncoding`;
  - `uname` currently mapped to raw `Tuple` (module install at line 912).
- [ ] `P1` Replace `_opcode` placeholder lists:
  - `get_intrinsic1_descs`, `get_intrinsic2_descs`, `get_special_method_names`, `get_nb_ops` currently mapped to generic `List` (module install at line 5538).
- [ ] `P1` Replace `subprocess._args_from_interpreter_flags` placeholder (`List`) with CPython-shaped behavior (module install at line 7351).
- [x] `P1` Replace `resource.getrlimit` placeholder (`Range`) with CPython-compatible return semantics (module install at line 7256):
  - wired `resource.getrlimit` to dedicated native builtin (`ResourceGetRLimit`) backed by host `getrlimit(2)`,
  - now returns CPython-shaped `(soft, hard)` tuple and raises `ValueError("invalid resource specified")` for invalid resource ids,
  - `RLIMIT_STACK`/`RLIM_INFINITY` bootstrap constants now come from host platform values instead of static placeholders.
- [ ] `P1` Replace `weakref` bootstrap placeholders:
  - `WeakSet` mapped to builtin `Set`;
  - weak dict types mapped to builtin `Dict` (module install at line 5599).
- [ ] `P1` Replace `_weakrefset.WeakSet` placeholder mapped to builtin `Set` (module install at line 5624).

## 2) Explicit Stub Modules

- [ ] `P0` Replace `_struct` bootstrap stub with CPython-compatible substrate or proper fallback strategy (`"pyrs _struct stub"`, module install at line 4786; stub doc at line 4800).
- [ ] `P1` Replace `_posixsubprocess` bootstrap stub with CPython-compatible substrate (`"pyrs _posixsubprocess stub"`, module install at line 7264; stub doc at line 7269).
- [ ] `P2` Decide final policy for `_testsinglephase` and `_testmultiphase` test modules in production bootstrap:
  - currently explicit stubs at lines 7383 and 7391.

## 3) Builtin-First Shadowing of CPython Pure-stdlib Modules

These modules are bootstrapped as builtins even though pure stdlib modules exist in CPython `Lib/`. This creates shadowing risk and divergence unless explicitly unloaded/replaced.

### 3.1 Not currently in `PURE_STDLIB_*` unload preference groups

- [ ] `P1` `platform` (line 912)
- [ ] `P1` `os` (line 1014)
- [ ] `P1` `_osx_support` (line 1536)
- [ ] `P1` `ssl` (line 1904)
- [ ] `P1` `codecs` (line 2605)
- [ ] `P1` `operator` (line 3439)
- [ ] `P1` `_colorize` (line 3675)
- [ ] `P1` `functools` (line 3722)
- [ ] `P1` `__future__` (line 4697)
- [ ] `P1` `weakref` (line 5599)
- [ ] `P1` `_weakrefset` (line 5624)
- [ ] `P1` `inspect` (line 5860)
- [ ] `P1` `io` (line 5923)
- [ ] `P1` `subprocess` (line 7351)
- [ ] `P1` `uuid` (line 7637)
- [ ] `P1` `asyncio` (line 7659)
- [ ] `P1` `threading` (line 7846)
- [ ] `P1` `signal` (line 7877)
- [ ] `P1` `abc` (line 8691)
- [ ] `P1` `sysconfig` (line 8725)
- [ ] `P1` `socket` (line 8768)

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
  - currently constructs fallback `FileFinder` class with `invalidate_caches = NoOp` (line 9529; `NoOp` use at line 9552).
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
