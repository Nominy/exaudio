// Stand-in for Babel's tRPC service, for a local end-to-end run of Exaudio's eval mode.
//
// Speaks the command-bridge protocol (`EXAUDIO_BABEL_SERVICE_COMMAND`): one request envelope
// (`{"operation", "payload"|"batchInput"}`) on stdin, the raw tRPC batch response on stdout.
// The eval responses follow the shapes the Babel dashboard bundle consumes (see
// tools/eval-parity); nothing here talks to Babel.
//
//   EVAL_SMOKE_STATE  JSON file holding the served session and every saved/submitted payload
//   EVAL_SMOKE_LOG    append-only JSONL of the requests (operation, payload, outcome)
//   EVAL_SMOKE_AUDIO  https URL served as every audio subject (default: an MDN sample mp3)
import { appendFileSync, closeSync, existsSync, openSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';

const statePath = process.env.EVAL_SMOKE_STATE;
const logPath = process.env.EVAL_SMOKE_LOG;
const audio = process.env.EVAL_SMOKE_AUDIO ?? 'https://raw.githubusercontent.com/mdn/webaudio-examples/main/audio-analyser/viper.mp3';
if (!statePath) {
  process.stderr.write('EVAL_SMOKE_STATE is required\n');
  process.exit(2);
}

export const EVAL_PROJECT_ID = '3882e77e-3ace-4ff2-b2df-290e3439cfce';
const Q = {
  preference: '6f1d2c4e-1a2b-4c3d-8e4f-000000000001',
  naturalA: '6f1d2c4e-1a2b-4c3d-8e4f-000000000002',
  correct: '6f1d2c4e-1a2b-4c3d-8e4f-000000000003',
  why: '6f1d2c4e-1a2b-4c3d-8e4f-000000000004',
  issues: '6f1d2c4e-1a2b-4c3d-8e4f-000000000005',
};

function session() {
  return {
    kind: 'session',
    sessionId: 'smoke-session-1',
    tasks: [
      {
        taskId: 'smoke-task-naturalness',
        format: 'tts-naturalness',
        formId: 'form-naturalness',
        formDisplayName: 'Naturalness (smoke)',
        formLinters: [],
        scenarioContext: { objective: null, domain: 'Phone', subdomain: null },
        subjects: [
          { id: 'subject-b', label: 'Clip B', position: 1, mediaUrl: audio, transcript: 'Second rendition' },
          { id: 'subject-a', label: 'Clip A', position: 0, mediaUrl: audio, transcript: 'First rendition' },
        ],
        questions: [
          { id: Q.naturalA, label: 'How natural is clip A?', caption: 'Optional', type: 'likert', sequenceNumber: 2,
            values: ['1', '2', '3', '4', '5'], valueLabels: ['Robotic', 'Poor', 'Fair', 'Good', 'Human'], metadata: { required: false }, linters: [] },
          { id: Q.preference, label: 'Which clip sounds more natural?', type: 'side_by_side', sequenceNumber: 1,
            values: ['a_better', 'tie', 'b_better'], valueLabels: ['A', 'Same', 'B'], metadata: {}, linters: [] },
        ],
      },
      {
        taskId: 'smoke-task-alpha',
        format: 'default',
        formId: 'form-alpha',
        formDisplayName: 'Alphanumerics (smoke)',
        formLinters: [],
        scenarioContext: { objective: 'read', domain: ' phone ', subdomain: 'numbers' },
        subjects: [
          { id: 'subject-audio', label: 'Recording', position: 0, mediaUrl: audio },
          { id: 'subject-written', label: 'Written form', position: 1, transcript: '04.03.' },
          { id: 'subject-spoken', label: 'Spoken form', position: 2, transcript: 'четвёртое марта' },
        ],
        questions: [
          { id: Q.correct, label: 'Was the script read correctly?', type: 'mcq', sequenceNumber: 1,
            values: ['yes', 'no'], valueLabels: ['Yes', 'No'], metadata: {}, linters: [] },
          { id: Q.why, label: 'What was wrong?', type: 'text', sequenceNumber: 2, values: [], valueLabels: [],
            metadata: { showWhen: { kind: 'answer', questionId: Q.correct, values: ['no'] } }, linters: [] },
          { id: Q.issues, label: 'Which issues apply?', type: 'multiselect', sequenceNumber: 3,
            values: ['digits', 'date', 'pause'], valueLabels: ['Digits', 'Date format', 'Pauses'],
            metadata: { required: false, showWhen: { kind: 'scenario', field: 'domain', value: 'PHONE' } }, linters: [] },
        ],
      },
    ],
  };
}

function readState() {
  return existsSync(statePath) ? JSON.parse(readFileSync(statePath, 'utf8')) : { saves: [], submits: [], holds: 0, completed: false };
}

function writeState(state) {
  writeFileSync(`${statePath}.tmp`, JSON.stringify(state, null, 2));
  renameSync(`${statePath}.tmp`, statePath);
}

const ok = (json) => ({ result: { data: { json } } });
const fail = (message, code = 'BAD_REQUEST') => ({ error: { json: { message, code, data: { code } } } });

function requiredAnswered(task, responses) {
  const byId = new Map(responses.map((response) => [response.formInputId, response]));
  const correct = byId.get(Q.correct)?.value;
  return task.questions.every((question) => {
    if (question.metadata.required === false) return true;
    if (question.id === Q.why && correct !== 'no') return !byId.has(Q.why);
    const response = byId.get(question.id);
    return response && response.value !== '';
  });
}

function handle(operation, payload, state) {
  switch (operation) {
    case 'worker.get':
      return ok({ id: 'smoke-worker' });
    case 'worker.getProjects':
      return ok([
        { id: EVAL_PROJECT_ID, name: 'RU-Alpha-Eval (smoke)', status: 'active', productKind: 'eval_session', category: null },
      ]);
    case 'evalTasks.serveSessionTasks':
      if (payload?.projectId !== EVAL_PROJECT_ID) return fail('Unknown project', 'NOT_FOUND');
      if (state.completed) return ok({ kind: 'empty', reason: 'all_at_consensus' });
      return ok(session());
    case 'evalTasks.holdSession':
      state.holds += 1;
      return ok(payload?.sessionId === 'smoke-session-1' && !state.completed);
    case 'evalTasks.saveProgress':
      state.saves.push(payload);
      return ok({ kind: 'saved' });
    case 'evalTasks.submitSession': {
      state.submits.push(payload);
      const tasks = session().tasks;
      const complete = payload?.sessionId === 'smoke-session-1'
        && tasks.every((task) => {
          const sent = payload.tasks?.find((entry) => entry.taskId === task.taskId);
          return sent && requiredAnswered(task, sent.inputResponses ?? []);
        });
      if (!complete) return ok({ kind: 'incomplete' });
      state.completed = true;
      return ok({ kind: 'completed' });
    }
    case 'project.getProjectInstructions':
      return ok({ id: 'smoke-instructions', instructions: '## How to do the task\n\nListen to every clip fully, then answer.\n\n| Written | Spoken |\n| --- | --- |\n| 04.03. | четвёртое марта |' });
    case 'rubric.getProjectRubricFlags':
      return ok([{ flagId: 'flag-1', name: 'naturalness', friendlyName: 'Naturalness', reviewCategory: 'content', type: 'manual', description: 'Does it sound like a person?' }]);
    case 'project.listProjectDocuments':
      return ok([]);
    default:
      return fail(`mock Babel: unsupported operation ${operation}`, 'NOT_FOUND');
  }
}

const envelope = JSON.parse(readFileSync(0, 'utf8'));
const operations = String(envelope.operation ?? '').split(',').map((operation) => operation.trim()).filter(Boolean);
const inputs = envelope.batchInput
  ? operations.map((_, index) => envelope.batchInput?.[index]?.json ?? envelope.batchInput?.[String(index)]?.json)
  : [envelope.payload];
// Queries never write; mutations hold an exclusive lock file around read-modify-write so
// concurrent bridge processes cannot overwrite each other's state.
const MUTATIONS = new Set(['evalTasks.holdSession', 'evalTasks.saveProgress', 'evalTasks.submitSession']);
const mutating = operations.some((operation) => MUTATIONS.has(operation));
const lockPath = `${statePath}.lock`;
let lock = null;
if (mutating) {
  const deadline = Date.now() + 10_000;
  while (lock === null) {
    try {
      lock = openSync(lockPath, 'wx');
    } catch (error) {
      if (error.code !== 'EEXIST' || Date.now() > deadline) throw error;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 25);
    }
  }
}
let response;
try {
  const state = readState();
  response = operations.map((operation, index) => handle(operation, inputs[index], state));
  if (mutating) writeState(state);
} finally {
  if (lock !== null) {
    closeSync(lock);
    unlinkSync(lockPath);
  }
}
if (logPath) {
  appendFileSync(logPath, `${JSON.stringify({ at: new Date().toISOString(), operations, payloads: inputs, outcome: response.map((item) => item.result?.data?.json?.kind ?? (item.error ? 'error' : 'ok')) })}\n`);
}
process.stdout.write(JSON.stringify(response));
