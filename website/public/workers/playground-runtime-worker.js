"use strict";

let runtimeModule = null;
let runtimeLoadPromise = null;
let replSession = null;
let lastRuntimeInfo = null;
let stdlibPackLoaded = false;
let stdlibPackPathLoaded = null;
let lastStdlibInfo = null;

const readField = (object, key) => {
  if (!object) return undefined;
  const value = object[key];
  if (typeof value === "function") {
    try {
      return value.call(object);
    } catch {
      return undefined;
    }
  }
  return value;
};

const normalizeRuntimeInfo = (info) => ({
  api_version: Number(readField(info, "api_version") || 0),
  pyrs_version: String(readField(info, "pyrs_version") || "dev"),
  cpython_compat_version: String(readField(info, "cpython_compat_version") || ""),
  supports_parse_compile: Boolean(readField(info, "supports_parse_compile")),
  supports_execution: Boolean(readField(info, "supports_execution")),
  execution_backend: String(readField(info, "execution_backend") || "wasm"),
  execution_status: String(readField(info, "execution_status") || "unknown"),
  execution_blocker_count: Number(readField(info, "execution_blocker_count") || 0),
});

const normalizeExecutionResult = (result) => ({
  success: Boolean(readField(result, "success")),
  phase: String(readField(result, "phase") || "runtime_error"),
  stdout: String(readField(result, "stdout") || ""),
  stderr: String(readField(result, "stderr") || ""),
  error: readField(result, "error") || null,
  blocker_key: readField(result, "blocker_key") || null,
  line: Number(readField(result, "line") || 0),
  column: Number(readField(result, "column") || 0),
});

const ensureReplSession = () => {
  if (!runtimeModule) return null;
  if (replSession) return replSession;
  if (typeof runtimeModule.WasmReplSession === "function") {
    replSession = new runtimeModule.WasmReplSession();
    return replSession;
  }
  if (typeof runtimeModule.WasmSession === "function") {
    replSession = new runtimeModule.WasmSession();
    return replSession;
  }
  return null;
};

const readContinuationPrompt = (session) => {
  if (!session) return false;
  return Boolean(readField(session, "continuation_prompt"));
};

const loadStdlibPack = async (stdlibPackPath) => {
  if (!runtimeModule) {
    return { ok: false, error: "runtime not loaded" };
  }
  if (
    stdlibPackLoaded &&
    stdlibPackPathLoaded === stdlibPackPath
  ) {
    return { ok: true, stdlibInfo: lastStdlibInfo };
  }
  if (!stdlibPackPath || typeof stdlibPackPath !== "string") {
    stdlibPackLoaded = true;
    stdlibPackPathLoaded = null;
    lastStdlibInfo = { pack_version: null, module_count: 0 };
    return { ok: true, stdlibInfo: lastStdlibInfo };
  }
  if (typeof runtimeModule.wasm_virtual_stdlib_register !== "function") {
    stdlibPackLoaded = true;
    stdlibPackPathLoaded = stdlibPackPath;
    lastStdlibInfo = { pack_version: null, module_count: 0 };
    return { ok: true, stdlibInfo: lastStdlibInfo };
  }

  const response = await fetch(stdlibPackPath, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`stdlib subset fetch failed (${response.status}) at ${stdlibPackPath}`);
  }
  const payload = await response.json();
  const modules = Array.isArray(payload?.modules) ? payload.modules : [];

  if (typeof runtimeModule.wasm_virtual_stdlib_clear === "function") {
    runtimeModule.wasm_virtual_stdlib_clear();
  }

  let registered = 0;
  for (const entry of modules) {
    if (!entry || typeof entry !== "object") continue;
    const moduleName = typeof entry.module === "string" ? entry.module : "";
    const sourceText = typeof entry.source === "string" ? entry.source : "";
    const isPackage = Boolean(entry.is_package);
    if (!moduleName || (!isPackage && !sourceText)) continue;
    const accepted = runtimeModule.wasm_virtual_stdlib_register(moduleName, sourceText, isPackage);
    if (accepted) {
      registered += 1;
    }
  }

  const runtimeCount =
    typeof runtimeModule.wasm_virtual_stdlib_count === "function"
      ? Number(runtimeModule.wasm_virtual_stdlib_count() || 0)
      : registered;
  stdlibPackLoaded = true;
  stdlibPackPathLoaded = stdlibPackPath;
  lastStdlibInfo = {
    pack_version: typeof payload?.pack_version === "string" ? payload.pack_version : null,
    module_count: runtimeCount,
  };
  return { ok: true, stdlibInfo: lastStdlibInfo };
};

const loadRuntime = async (wasmEntrypoint, stdlibPackPath) => {
  if (runtimeModule) {
    const stdlib = await loadStdlibPack(stdlibPackPath);
    if (!stdlib.ok) {
      return stdlib;
    }
    return {
      ok: true,
      runtimeInfo: lastRuntimeInfo,
      stdlibInfo: lastStdlibInfo,
      prompt_continuation: readContinuationPrompt(ensureReplSession()),
    };
  }
  if (runtimeLoadPromise) {
    return runtimeLoadPromise;
  }

  runtimeLoadPromise = (async () => {
    const moduleRef = await import(wasmEntrypoint);
    if (typeof moduleRef.default === "function") {
      await moduleRef.default();
    }
    if (typeof moduleRef.init_wasm_runtime === "function") {
      moduleRef.init_wasm_runtime();
    }

    runtimeModule = moduleRef;
    replSession = null;
    stdlibPackLoaded = false;
    stdlibPackPathLoaded = null;
    lastStdlibInfo = null;
    await loadStdlibPack(stdlibPackPath);
    ensureReplSession();

    lastRuntimeInfo =
      typeof runtimeModule.wasm_runtime_info === "function"
        ? normalizeRuntimeInfo(runtimeModule.wasm_runtime_info())
        : null;
    return {
      ok: true,
      runtimeInfo: lastRuntimeInfo,
      stdlibInfo: lastStdlibInfo,
      prompt_continuation: readContinuationPrompt(ensureReplSession()),
    };
  })()
    .catch((error) => {
      runtimeModule = null;
      replSession = null;
      lastRuntimeInfo = null;
      stdlibPackLoaded = false;
      stdlibPackPathLoaded = null;
      lastStdlibInfo = null;
      const message = error instanceof Error ? error.message : String(error);
      return { ok: false, error: message };
    })
    .finally(() => {
      runtimeLoadPromise = null;
    });

  return runtimeLoadPromise;
};

const executeSource = (source) => {
  if (!runtimeModule) {
    return { ok: false, error: "runtime not loaded" };
  }
  try {
    const session = ensureReplSession();
    let result;
    if (session && typeof session.execute_input === "function") {
      result = session.execute_input(source);
    } else if (session && typeof session.execute === "function") {
      result = session.execute(source);
    } else if (typeof runtimeModule.execute === "function") {
      result = runtimeModule.execute(source);
    } else {
      return { ok: false, error: "Runtime execute entrypoint is unavailable." };
    }
    return {
      ok: true,
      result: normalizeExecutionResult(result),
      prompt_continuation: readContinuationPrompt(session),
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { ok: false, error: message };
  }
};

const resetSession = () => {
  if (!runtimeModule) {
    return { ok: true };
  }
  try {
    const session = ensureReplSession();
    if (session && typeof session.reset === "function") {
      session.reset();
    } else {
      replSession = null;
      ensureReplSession();
    }
    return {
      ok: true,
      prompt_continuation: readContinuationPrompt(ensureReplSession()),
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { ok: false, error: message };
  }
};

self.addEventListener("message", async (event) => {
  const payload = event.data;
  const requestId =
    payload && Number.isFinite(Number(payload.requestId)) ? Number(payload.requestId) : null;
  const action = payload && typeof payload.action === "string" ? payload.action : "";

  let response;
  if (action === "load") {
    response = await loadRuntime(payload.wasmEntrypoint, payload.stdlibPackPath);
  } else if (action === "execute") {
    response = executeSource(typeof payload.source === "string" ? payload.source : "");
  } else if (action === "reset") {
    response = resetSession();
  } else {
    response = { ok: false, error: `unknown playground worker action: ${action}` };
  }

  self.postMessage({
    requestId,
    ...response,
  });
});
