// End-to-end eval smoke against the real exaudio-relay whose Babel upstream is
// tools/eval-smoke/mock-babel.mjs, signed in as a relay sub-account.
//
//   node tools/eval-smoke/run.mjs [--out <dir>] [--no-build] [--gui [--audible]]
//
// Default (headless): the `eval_e2e` example drives the eval client code the screen uses
// (dashboard, resources, serve, owner take-over refused, clip download and decode, answers,
// drafts, save, hold, submit) without a window or desktop input. --gui instead opens the app
// on the same relay for a manual pass (audio silent unless --audible).
// Nothing touches the user's data: everything lives in <out>, and the mock never reaches Babel.
// Progress: <out>/status.json (atomic) and <out>/events.jsonl; logs: build.log, relay.log,
// e2e.log or app.log, mock-requests.jsonl, mock-state.json (every saved/submitted payload).
// Graceful stop (--gui): create <out>/STOP. A run is `completed` (exit 0) only when the mock
// confirmed a completed submission. Every run starts a fresh session in <out>.
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { appendFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { createWriteStream } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../..');
const argv = process.argv.slice(2);
const flag = (name) => argv.includes(`--${name}`);
const option = (name) => (argv.includes(`--${name}`) ? argv[argv.indexOf(`--${name}`) + 1] : undefined);
const out = resolve(option('out') ?? join(repo, 'target/eval-smoke'));
mkdirSync(out, { recursive: true });
const paths = {
  status: join(out, 'status.json'),
  events: join(out, 'events.jsonl'),
  stop: join(out, 'STOP'),
  state: join(out, 'mock-state.json'),
  requests: join(out, 'mock-requests.jsonl'),
  appdata: join(out, 'appdata'),
  relayStore: join(out, 'relay-store.json'),
};
const started = Date.now();
let status = { state: 'initializing', stage: 'prepare' };

function event(kind, detail = {}) {
  appendFileSync(paths.events, `${JSON.stringify({ at: new Date().toISOString(), kind, ...detail })}\n`);
}
function setStatus(patch) {
  status = { ...status, ...patch, elapsedSeconds: Math.round((Date.now() - started) / 1000), at: new Date().toISOString() };
  writeFileSync(`${paths.status}.tmp`, JSON.stringify(status, null, 2));
  renameSync(`${paths.status}.tmp`, paths.status);
}
function log(line) {
  console.log(`${new Date().toISOString().slice(11, 19)} ${line}`);
}
function run(command, args, logName, env = {}) {
  return new Promise((resolveRun, reject) => {
    const sink = createWriteStream(join(out, logName), { flags: 'a' });
    const child = spawn(command, args, { cwd: repo, env: { ...process.env, ...env }, windowsHide: true });
    child.stdout.on('data', (chunk) => { sink.write(chunk); process.stdout.write(chunk); });
    child.stderr.on('data', (chunk) => { sink.write(chunk); process.stdout.write(chunk); });
    child.on('error', reject);
    child.on('close', (code) => { sink.end(); code === 0 ? resolveRun() : reject(new Error(`${command} exited ${code}`)); });
  });
}

const children = [];
function stopChildren() {
  for (const child of children) if (child.exitCode === null) child.kill();
}
process.on('SIGINT', () => { stopChildren(); process.exit(130); });

try {
  rmSync(paths.stop, { force: true });
  // A run is always a fresh session: mock state, relay store (sub-accounts and eval
  // bindings) and app data start empty together.
  for (const path of [paths.state, `${paths.state}.lock`, paths.requests, paths.relayStore, paths.appdata]) {
    rmSync(path, { force: true, recursive: true });
  }
  event('started', { out });

  const gui = flag('gui');
  if (!flag('no-build')) {
    const build = gui
      ? ['build', '--locked', '-p', 'exaudio-app', '-p', 'exaudio-babel-relay']
      : ['build', '--locked', '-p', 'exaudio-babel-relay', '-p', 'exaudio-babel-services', '--bin', 'exaudio-relay', '--example', 'eval_e2e'];
    setStatus({ state: 'running', stage: 'build 1/3', detail: `cargo ${build.join(' ')}` });
    log('building (cargo output in build.log)');
    await run('cargo', build, 'build.log');
    event('built');
  }
  const exe = (name) => join(repo, 'target/debug', process.platform === 'win32' ? `${name}.exe` : name);

  setStatus({ state: 'running', stage: 'relay 2/3' });
  const ownerToken = randomBytes(24).toString('base64url');
  const mock = `node "${join(here, 'mock-babel.mjs')}"`;
  const relayLog = createWriteStream(join(out, 'relay.log'), { flags: 'a' });
  // A port chosen up front and an HTTP probe: readiness does not depend on catching one stderr line.
  const port = await new Promise((resolvePort) => {
    const probe = createServer().listen(0, '127.0.0.1', () => { const { port: free } = probe.address(); probe.close(() => resolvePort(free)); });
  });
  const relayUrl = `http://127.0.0.1:${port}`;
  const relay = spawn(exe('exaudio-relay'), ['--bind', `127.0.0.1:${port}`, '--store', paths.relayStore, '--babel-service-command', mock], {
    // The owner token travels in the environment, never in argv.
    env: { ...process.env, EVAL_SMOKE_STATE: paths.state, EVAL_SMOKE_LOG: paths.requests, EXAUDIO_SUBACCOUNT_OWNER_TOKEN: ownerToken },
    windowsHide: true,
  });
  children.push(relay);
  relay.stdout.pipe(relayLog);
  relay.stderr.pipe(relayLog);
  const readyBy = Date.now() + 120_000;
  for (;;) {
    if (relay.exitCode !== null) throw new Error(`relay exited ${relay.exitCode}`);
    const probe = await fetch(`${relayUrl}/api/subaccounts/owner/dashboard`, { headers: { authorization: `Bearer ${ownerToken}` } }).catch(() => null);
    if (probe?.ok) break;
    if (Date.now() > readyBy) throw new Error('relay did not answer within 120 s');
    await new Promise((resume) => setTimeout(resume, 250));
  }
  event('relay-ready', { relayUrl, afterMs: Date.now() - started });
  log(`relay at ${relayUrl}`);

  const owner = { authorization: `Bearer ${ownerToken}`, 'content-type': 'application/json' };
  const created = await (await fetch(`${relayUrl}/api/subaccounts/owner/subaccounts`, { method: 'POST', headers: owner, body: JSON.stringify({ label: 'Eval smoke' }) })).json();
  const subaccount = created.subaccounts.at(-1);
  const assigned = await fetch(`${relayUrl}/api/subaccounts/owner/subaccounts/${subaccount.id}/projects`, {
    method: 'PUT', headers: owner, body: JSON.stringify({ projectIds: ['3882e77e-3ace-4ff2-b2df-290e3439cfce'] }),
  });
  if (!assigned.ok) throw new Error(`assigning the project failed: HTTP ${assigned.status}`);
  const login = await (await fetch(`${relayUrl}/api/subaccounts/login`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: subaccount.code }) })).json();
  const accountDir = join(paths.appdata, 'Exaudio', 'providers', 'exaudio.babel.account', 'eval-smoke');
  mkdirSync(accountDir, { recursive: true });
  writeFileSync(join(accountDir, 'subaccount.json'), JSON.stringify({ hostUrl: relayUrl, token: login.session.token, subaccountId: login.session.subaccountId, displayName: 'Eval smoke' }));
  writeFileSync(join(accountDir, 'account.json'), JSON.stringify({ id: 'eval-smoke', label: `Eval smoke @ ${relayUrl.replace('http://', '')}`, kind: 'relay', identifier: relayUrl }));
  event('subaccount-ready', { subaccountId: subaccount.id });

  if (!gui) {
    // Headless: the eval client code the screen uses, no window, no input on the desktop.
    setStatus({ state: 'running', stage: 'e2e 3/3', relayUrl });
    const e2eLog = join(out, 'e2e.log');
    rmSync(e2eLog, { force: true });
    let e2eFailed = false;
    await run(join(repo, 'target/debug/examples', process.platform === 'win32' ? 'eval_e2e.exe' : 'eval_e2e'), [], 'e2e.log', {
      EVAL_E2E_RELAY: relayUrl,
      EVAL_E2E_TOKEN: login.session.token,
      EVAL_E2E_OWNER_TOKEN: ownerToken,
    }).catch(() => { e2eFailed = true; });
    stopChildren();
    const final = existsSync(paths.state) ? JSON.parse(readFileSync(paths.state, 'utf8')) : null;
    const passed = !e2eFailed && final?.completed === true;
    const outcome = passed ? 'completed' : 'failed';
    setStatus({ state: outcome, stage: 'done', saves: final?.saves.length ?? 0, submits: final?.submits.length ?? 0, completed: final?.completed ?? false });
    event(outcome, { sessionCompleted: final?.completed ?? false, e2eFailed });
    log(`done (${outcome}); steps in e2e.log, payloads in mock-state.json`);
    process.exit(passed ? 0 : 1);
  }

  setStatus({ state: 'running', stage: 'app 3/3', detail: 'Open Start -> Babel -> RU-Alpha-Eval (smoke); close the window to finish', relayUrl });
  const app = spawn(exe('exaudio-app'), [], {
    env: {
      ...process.env,
      APPDATA: paths.appdata,
      EXAUDIO_PROVIDER_DIR: join(paths.appdata, 'Exaudio', 'providers'),
      EXAUDIO_SUBACCOUNT_RELAY_SETTINGS: join(out, 'relay-settings.json'),
      // Anything that is not the relay goes nowhere: no real Babel, Clerk or review backend.
      EXAUDIO_BABEL_SERVICE_BASE_URL: 'http://127.0.0.1:9',
      EXAUDIO_BABEL_CLERK_URL: 'http://127.0.0.1:9',
      EXAUDIO_REVIEW_BACKEND_BASE_URL: 'http://127.0.0.1:9',
      // Full playback path, silent output (shared machines); `--audible` to hear the clips.
      ...(flag('audible') ? {} : { EXAUDIO_AUDIO_SILENT: '1' }),
    },
    windowsHide: false,
  });
  children.push(app);
  const appLog = createWriteStream(join(out, 'app.log'), { flags: 'a' });
  app.stdout.pipe(appLog);
  app.stderr.pipe(appLog);
  event('app-started', { pid: app.pid });
  setStatus({ appPid: app.pid });
  log(`app pid ${app.pid}; data in ${paths.appdata}`);

  let lastRequests = -1;
  const exitCode = await new Promise((resolveExit) => {
    const timer = setInterval(() => {
      if (existsSync(paths.stop)) {
        event('stopping', { reason: 'STOP file' });
        setStatus({ state: 'stopping' });
        app.kill();
      }
      const mockState = existsSync(paths.state) ? JSON.parse(readFileSync(paths.state, 'utf8')) : { saves: [], submits: [], holds: 0, completed: false };
      const requests = existsSync(paths.requests) ? readFileSync(paths.requests, 'utf8').trim().split('\n').filter(Boolean).length : 0;
      if (requests !== lastRequests) {
        lastRequests = requests;
        const last = requests ? JSON.parse(readFileSync(paths.requests, 'utf8').trim().split('\n').at(-1)) : null;
        log(`mock requests ${requests}; last ${last ? `${last.operations.join(',')} -> ${last.outcome.join(',')}` : '-'}; saves ${mockState.saves.length}; submits ${mockState.submits.length}; completed ${mockState.completed}`);
      }
      setStatus({ requests, saves: mockState.saves.length, submits: mockState.submits.length, holds: mockState.holds, completed: mockState.completed, lastActivity: new Date().toISOString() });
    }, 2000);
    app.on('close', (code) => { clearInterval(timer); resolveExit(code); });
  });
  event('app-exited', { code: exitCode });
  stopChildren();
  const final = existsSync(paths.state) ? JSON.parse(readFileSync(paths.state, 'utf8')) : null;
  const stopped = existsSync(paths.stop);
  const passed = !stopped && exitCode === 0 && final?.completed === true;
  // Passing needs a clean app exit after Babel (the mock) confirmed a completed submission.
  const outcome = stopped ? 'stopped' : passed ? 'completed' : 'unfinished';
  setStatus({ state: outcome, stage: 'done', exitCode, completed: final?.completed ?? false });
  event(outcome, { sessionCompleted: final?.completed ?? false, exitCode });
  log(`done (${outcome}); session completed by the mock: ${final?.completed ?? false}; app exit ${exitCode}`);
  if (!passed) process.exitCode = 1;
} catch (error) {
  stopChildren();
  setStatus({ state: 'failed', error: String(error?.message ?? error) });
  event('failed', { error: String(error?.message ?? error) });
  log(`failed: ${error?.message ?? error}`);
  process.exitCode = 1;
}
