// Eval parity: Babel's own eval-form code vs Exaudio's `babel::eval` on a generated corpus.
//
//   node tools/eval-parity/run.mjs [--chunks <dir>] [--cases N] [--seed S] [--out <dir>]
//
// --chunks (or BABEL_DASHBOARD_CHUNKS) names a local dump of the Babel dashboard's
// `static/chunks/*.js`; the code is proprietary and never committed. For every case both sides
// report the visible questions, the questions offering a comment and the `inputResponses` Babel
// would send (or that submission is refused). Exaudio's side is
// `cargo run -p exaudio-babel-services --example eval_parity`; build it first when the run must
// stay short (`cargo build -p exaudio-babel-services --example eval_parity --locked`).
// Writes <out>/corpus.json, <out>/report.json and exits 1 on any difference.
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { generateCorpus } from './corpus.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../..');
const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, arg, index, all) => (arg.startsWith('--') ? [...pairs, [arg.slice(2), all[index + 1]]] : pairs), []),
);
const chunks = args.chunks ?? process.env.BABEL_DASHBOARD_CHUNKS;
if (!chunks) {
  console.error('Pass --chunks <dir> or set BABEL_DASHBOARD_CHUNKS to a dump of the Babel dashboard chunks.');
  process.exit(2);
}
const out = resolve(args.out ?? join(repo, 'target/eval-parity'));
mkdirSync(out, { recursive: true });

const { loadBabelEval, evaluateTask } = createRequire(import.meta.url)('./babel.cjs');
const babel = loadBabelEval(resolve(chunks));
console.log(`Babel modules: ${JSON.stringify(babel.modules)}`);

const corpus = generateCorpus({ seed: Number(args.seed ?? 20261126), cases: Number(args.cases ?? 2000) });
const corpusPath = join(out, 'corpus.json');
writeFileSync(corpusPath, JSON.stringify(corpus));

const expected = corpus.map((item) => ({ id: item.id, ...evaluateTask(babel, item.task, item.answers, item.comments) }));

const cargo = spawnSync(
  'cargo',
  ['run', '-q', '--locked', '-p', 'exaudio-babel-services', '--example', 'eval_parity', '--', corpusPath],
  { cwd: repo, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
);
if (cargo.status !== 0) {
  console.error(cargo.stderr);
  process.exit(cargo.status ?? 1);
}
const actual = new Map(JSON.parse(cargo.stdout).map((item) => [item.id, item]));

// Key order is not part of the JSON contract.
const canonical = (value) => JSON.stringify(value, (_, v) => (v && typeof v === 'object' && !Array.isArray(v)
  ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b)))
  : v));

const differences = [];
for (const babelCase of expected) {
  const ours = actual.get(babelCase.id);
  const fields = {
    visible: [babelCase.visible, ours?.visible],
    commentable: [babelCase.commentable, ours?.commentable],
    refused: [babelCase.error !== null, ours?.refused],
    responses: [babelCase.responses, ours?.responses ?? null],
  };
  const differing = Object.entries(fields).filter(([, [left, right]]) => canonical(left) !== canonical(right));
  if (differing.length > 0) {
    differences.push({
      id: babelCase.id,
      fields: Object.fromEntries(differing.map(([name, [babelValue, exaudioValue]]) => [name, { babel: babelValue, exaudio: exaudioValue }])),
      babelError: babelCase.error,
      exaudioError: ours?.error ?? null,
    });
  }
}

const refused = expected.filter((item) => item.error !== null).length;
const summary = {
  cases: corpus.length,
  matching: corpus.length - differences.length,
  differing: differences.length,
  babelRefused: refused,
  babelModules: babel.modules,
};
writeFileSync(join(out, 'report.json'), JSON.stringify({ summary, differences }, null, 2));
console.log(JSON.stringify(summary));
for (const difference of differences.slice(0, 5)) console.log(JSON.stringify(difference));
process.exit(differences.length === 0 ? 0 : 1);
