// Babel's own eval-form code, loaded from a dump of the dashboard's Next.js chunks.
//
// The chunks are Babel's proprietary web code: they are never committed or published. Point
// BABEL_DASHBOARD_CHUNKS at a local directory of `static/chunks/*.js` files (any file names).
//
// Nothing here re-implements Babel: modules are located by content signature, executed through a
// minimal webpack `require`, and the eval form's module-local visibility rule is cut out of the
// component source verbatim and evaluated against the module's own zod import.
'use strict';
const fs = require('node:fs');
const path = require('node:path');

function loadFactories(dir) {
  const factories = {};
  const previous = globalThis.webpackChunk_N_E;
  globalThis.self = globalThis;
  globalThis.webpackChunk_N_E = { push([, modules]) { Object.assign(factories, modules); } };
  try {
    for (const file of fs.readdirSync(dir).sort()) {
      if (!file.endsWith('.js') || /webpack-|polyfills/.test(file)) continue;
      // Each chunk only registers factories; nothing runs until required.
      (0, eval)(fs.readFileSync(path.join(dir, file), 'utf8'));
    }
  } finally {
    globalThis.webpackChunk_N_E = previous;
  }
  return factories;
}

function makeRequire(factories) {
  const cache = {};
  const req = (id) => {
    if (cache[id]) return cache[id].exports;
    const factory = factories[id];
    if (!factory) throw new Error(`Babel module ${id} is not in the dump`);
    const module = (cache[id] = { id, loaded: false, exports: {} });
    factory.call(module.exports, module, module.exports, req);
    module.loaded = true;
    return module.exports;
  };
  req.d = (exports, getters) => {
    for (const key in getters) {
      if (!Object.prototype.hasOwnProperty.call(exports, key)) {
        Object.defineProperty(exports, key, { enumerable: true, get: getters[key] });
      }
    }
  };
  req.o = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  req.r = (exports) => Object.defineProperty(exports, '__esModule', { value: true });
  req.n = (module) => {
    const getter = module && module.__esModule ? () => module.default : () => module;
    req.d(getter, { a: getter });
    return getter;
  };
  req.t = (value) => value;
  req.g = globalThis;
  req.p = '/_next/';
  req.e = () => Promise.resolve();
  req.nmd = (module) => module;
  req.hmd = (module) => module;
  return req;
}

// The single factory whose source matches every pattern.
function findModule(factories, label, patterns) {
  const hits = Object.entries(factories).filter(([, factory]) => {
    const source = factory.toString();
    return patterns.every((pattern) => pattern.test(source));
  });
  if (hits.length !== 1) {
    throw new Error(`${label}: expected one Babel module, found ${hits.length}; the dump changed shape`);
  }
  return { id: hits[0][0], source: hits[0][1].toString() };
}

// `source.slice(start)` up to the bracket that closes the first opener at or after `start`.
function balanced(source, start, open, close) {
  const first = source.indexOf(open, start);
  let depth = 0;
  for (let i = first; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === '"' || ch === "'" || ch === '`') {
      for (i += 1; i < source.length && source[i] !== ch; i += 1) if (source[i] === '\\') i += 1;
      continue;
    }
    if (ch === open) depth += 1;
    else if (ch === close && (depth -= 1) === 0) return source.slice(start, i + 1);
  }
  throw new Error('unbalanced Babel source');
}

function loadBabelEval(dir) {
  globalThis.window = globalThis.window || globalThis;
  globalThis.document = globalThis.document || {
    createElement: () => ({ setAttribute() {}, appendChild() {}, style: {} }),
    head: { appendChild() {} },
    documentElement: { style: {} },
    querySelectorAll: () => [],
    querySelector: () => null,
    getElementsByTagName: () => [],
    addEventListener() {},
    removeEventListener() {},
  };
  const factories = loadFactories(dir);
  const req = makeRequire(factories);

  // Question-type adapters: { Component, validate, buildResponse } per form_input_type.
  const adapters = findModule(factories, 'question adapters', [/buildResponse/, /__none_of_the_above__/, /selectedValues/]);
  const { ae: adapterFor, v3: hasAnswer } = req(adapters.id);
  // `metadata.required === false` (zod literal) marks an optional question.
  const optional = findModule(factories, 'optional-question rule', [/required:[\w$]+\.i0\(!1\)/]);
  const isOptional = Object.values(req(optional.id)).find((value) => typeof value === 'function');
  // Comment box rule: `metadata.allowComment`, else the choice types.
  const comments = findModule(factories, 'comment rule', [/allowComment:[\w$]+\.O7\(\)/]);
  const allowsComment = (() => {
    const exports = req(comments.id);
    const name = /([\w$]+)=[\w$]+\.Ry\(\{allowComment:[\w$]+\.O7\(\)\}\);function ([\w$]+)\(/.exec(comments.source)[2];
    const exportKey = new RegExp(`([\\w$]+):function\\(\\)\\{return ${name.replace('$', '\\$')}\\}`).exec(comments.source)[1];
    return exports[exportKey];
  })();

  // Visibility: module-local in the eval form component.
  const form = findModule(factories, 'eval form', [/Answer:"answer",Scenario:"scenario"/, /showWhen/]);
  const schemaStatement = /let ([\w$]+)=\{Answer:"answer",Scenario:"scenario"\},([\w$]+)=([\w$]+)\.G0\(/.exec(form.source);
  const [, enumName, schemaName, zodName] = schemaStatement;
  const zodId = new RegExp(`var ${zodName.replace('$', '\\$')}=[\\w$]+\\((\\d+)\\)`).exec(form.source)[1];
  const schemaSource = balanced(form.source, schemaStatement.index + schemaStatement[0].length - 1, '(', ')');
  const normalize = /function ([\w$]+)\(e\)\{return\(null==e\?void 0:e\.trim\(\)\.toLowerCase\(\)\)\|\|null\}/.exec(form.source);
  const predicateStart = form.source.indexOf('function(e,t){let n=function(e){let t="object"!=typeof e');
  if (predicateStart < 0) throw new Error('eval form visibility predicate not found');
  const predicateSource = balanced(form.source, predicateStart, '{', '}');
  const isVisible = new Function(
    zodName,
    `let ${enumName}={Answer:"answer",Scenario:"scenario"};` +
      `let ${schemaName}=${zodName}.G0${schemaSource};` +
      `${normalize[0]};` +
      `return ${predicateSource};`,
  )(req(zodId));

  return {
    modules: { adapters: adapters.id, optional: optional.id, comments: comments.id, form: form.id },
    adapterFor,
    hasAnswer,
    isOptional,
    allowsComment,
    isVisible,
  };
}

// The form as Babel renders and submits one task: hidden answers are dropped until the visible
// set is stable (the `eq` effect), then the submit path validates and builds the responses.
function evaluateTask(babel, task, answers, comments) {
  const questions = [...task.questions].sort((a, b) => a.sequenceNumber - b.sequenceNumber);
  const scenario = task.scenarioContext ?? { objective: null, domain: null, subdomain: null };
  let values = { ...answers };
  let freeTexts = { ...comments };
  let visible;
  for (let round = 0; round <= questions.length; round += 1) {
    const arrays = Object.fromEntries(Object.entries(values).filter(([, value]) => Array.isArray(value)));
    visible = questions.filter((question) => babel.isVisible(question.metadata, { answers: arrays, scenario }));
    const ids = new Set(visible.map((question) => question.id));
    const keep = (map) => Object.fromEntries(Object.entries(map).filter(([id]) => ids.has(id)));
    const next = keep(values);
    const nextTexts = keep(freeTexts);
    const stable = Object.keys(next).length === Object.keys(values).length
      && Object.keys(nextTexts).length === Object.keys(freeTexts).length;
    values = next;
    freeTexts = nextTexts;
    if (stable) break;
  }
  const text = (id) => freeTexts[id]?.trim() || null;
  let responses = null;
  let error = null;
  try {
    // The form only enables submit with at least one visible question (`G2` in the form).
    if (visible.length === 0) throw new Error('no visible questions');
    responses = visible
      .filter((question) => !babel.isOptional(question.metadata) || babel.hasAnswer(values[question.id] ?? [], text(question.id)))
      .map((question) => {
        const adapter = babel.adapterFor(question);
        const value = values[question.id] ?? [];
        const verdict = adapter.validate(value, question);
        if (!verdict.valid) throw new Error(verdict.message);
        return adapter.buildResponse({ question, value, freeText: text(question.id), task });
      });
  } catch (cause) {
    error = cause.message;
  }
  return {
    visible: visible.map((question) => question.id),
    commentable: visible.filter((question) => babel.allowsComment(question)).map((question) => question.id),
    responses,
    error,
  };
}

module.exports = { loadBabelEval, evaluateTask };
