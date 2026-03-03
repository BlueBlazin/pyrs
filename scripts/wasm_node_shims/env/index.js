"use strict";

const warned = new Set();

function warnOnce(name) {
  if (warned.has(name)) {
    return;
  }
  warned.add(name);
  // eslint-disable-next-line no-console
  console.warn(
    `[pyrs wasm] node env shim import '${name}' is not available; returning fallback value.`,
  );
}

module.exports = new Proxy(
  {},
  {
    get(_target, prop) {
      if (typeof prop !== "string") {
        return undefined;
      }
      return (..._args) => {
        warnOnce(prop);
        return 0;
      };
    },
  },
);
