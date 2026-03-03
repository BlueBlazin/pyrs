"use strict";

let runtimeModule = null;
let runtimeLoadPromise = null;
let replSession = null;
let lastRuntimeInfo = null;

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

const loadRuntime = async (wasmEntrypoint) => {
  if (runtimeModule) {
    return {
      ok: true,
      runtimeInfo: lastRuntimeInfo,
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
    ensureReplSession();

    lastRuntimeInfo =
      typeof runtimeModule.wasm_runtime_info === "function"
        ? normalizeRuntimeInfo(runtimeModule.wasm_runtime_info())
        : null;
    return {
      ok: true,
      runtimeInfo: lastRuntimeInfo,
      prompt_continuation: readContinuationPrompt(ensureReplSession()),
    };
  })()
    .catch((error) => {
      runtimeModule = null;
      replSession = null;
      lastRuntimeInfo = null;
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
    response = await loadRuntime(payload.wasmEntrypoint);
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
