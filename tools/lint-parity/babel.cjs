// Babel's native linter (Gold), run in Node from the recovered Babel editor bundle that ships in
// the Babel checkout's e2e fixtures. Module 72857 holds the linter class and its helpers,
// module 78100 the per-project linter configuration. Only the modules the linter needs execute;
// every other module (React, UI, tRPC) is an inert stub.
const fs = require('fs');
const path = require('path');

const MODULE_SOURCES = 'shared/babel-extension-platform/packages/babel-extension-e2e/fixtures/recreation/app/src/recovered/generated/moduleSources.ts';
const REAL_MODULES = new Set(['78100', '71241', '62252', '3171', '64647', '96890', '43781', '22649']);
// Module-level bindings of 72857 the generator exports as data.
const EXPORTED = ['e3', 'e8', 'e6', 'eZ', 'eG', 'eH', 'eV', 'eq', 'eY', 'tw', 'ty', 't_'];

function readModuleSources(babelDir) {
  const text = fs.readFileSync(path.join(babelDir, MODULE_SOURCES), 'utf8');
  const start = text.indexOf('{', text.indexOf('recoveredModuleSources'));
  let depth = 0;
  let inString = false;
  for (let index = start; index < text.length; index += 1) {
    const character = text[index];
    if (inString) {
      if (character === '\\') index += 1;
      else if (character === '"') inString = false;
    } else if (character === '"') inString = true;
    else if (character === '{') depth += 1;
    else if (character === '}' && --depth === 0) return JSON.parse(text.slice(start, index + 1));
  }
  throw new Error('unterminated recoveredModuleSources');
}

function stub() {
  const value = new Proxy(function () {}, {
    get(target, key) {
      if (key === '__esModule') return true;
      if (key === Symbol.iterator) return function* () {};
      if (key === Symbol.toPrimitive) return () => '';
      if (key === 'length') return 0;
      if (key === 'then') return undefined;
      if (key === 'prototype') return {};
      return value;
    },
    apply: () => value,
    construct: () => value,
  });
  return value;
}

function loadBabel(babelDir) {
  const sources = readModuleSources(babelDir);
  const cache = {};
  function req(id) {
    id = String(id);
    if (cache[id]) return cache[id].exports;
    if (!REAL_MODULES.has(id) || !sources[id]) return stub();
    const module = { exports: {} };
    cache[id] = module;
    (0, eval)(`(${sources[id]})`).call(module.exports, module, module.exports, req);
    return module.exports;
  }
  req.d = (exports, getters) => {
    for (const key in getters) {
      if (!Object.prototype.hasOwnProperty.call(exports, key)) Object.defineProperty(exports, key, { enumerable: true, get: getters[key] });
    }
  };
  req.r = (exports) => Object.defineProperty(exports, '__esModule', { value: true });
  req.n = (module) => {
    const getter = module && module.__esModule ? () => module.default : () => module;
    req.d(getter, { a: getter });
    return getter;
  };
  req.o = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  req.g = globalThis;
  req.U = function () { return stub(); };
  req.p = '';
  req.b = 'file:///';
  req.tu = (value) => value;

  const editor = sources['72857'];
  const exportList = EXPORTED.map((name) => `${JSON.stringify(name)}:${name}`).join(',');
  const patched = `${editor.slice(0, editor.lastIndexOf('}'))};globalThis.__babelLint={Linter:tR,data:{${exportList}}};}`;
  const module = { exports: {} };
  (0, eval)(`(${patched})`).call(module.exports, module, module.exports, req);
  const { Linter, data } = globalThis.__babelLint;
  return { Linter, data, config: req('78100'), languageCodes: req('64647')._ };
}

module.exports = { loadBabel };
