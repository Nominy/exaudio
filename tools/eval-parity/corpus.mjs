// Deterministic corpus of Babel evaluation tasks and answer sets for the parity run.
//
// Every task stays inside what Exaudio accepts (supported question types, valid showWhen,
// no linters), so a difference against Babel is a semantic difference, not a refusal.
// Answers deliberately include hidden-question leftovers, chained conditions, whitespace
// comments, optional questions answered only by a comment and missing required answers.

const TYPES = ['mcq', 'likert', 'dropdown', 'side_by_side', 'multiselect', 'text'];
const FIELDS = ['objective', 'domain', 'subdomain'];
const WORDS = ['phone', 'Dates', 'addresses', 'names', 'money', 'Time'];

function rng(seed) {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 0x100000000;
  };
}

function uuid(next) {
  const hex = () => Math.floor(next() * 16).toString(16);
  const part = (n) => Array.from({ length: n }, hex).join('');
  return `${part(8)}-${part(4)}-4${part(3)}-8${part(3)}-${part(12)}`;
}

function spaced(next, word) {
  const cased = next() < 0.5 ? word.toUpperCase() : word.toLowerCase();
  return `${next() < 0.3 ? '  ' : ''}${cased}${next() < 0.3 ? ' ' : ''}`;
}

// Babel only renders a comment box here; answers elsewhere cannot carry one.
function commentable(question) {
  const allow = question.metadata.allowComment;
  return typeof allow === 'boolean' ? allow : question.type !== 'text';
}

export function generateCorpus({ seed = 20261126, cases = 2000 } = {}) {
  const next = rng(seed);
  const pick = (items) => items[Math.floor(next() * items.length)];
  const corpus = [];
  for (let index = 0; index < cases; index += 1) {
    const questions = [];
    const count = 1 + Math.floor(next() * 6);
    for (let q = 0; q < count; q += 1) {
      const type = pick(TYPES);
      const values = type === 'text' ? [] : Array.from({ length: 2 + Math.floor(next() * 4) }, (_, i) => `v${i}`);
      const metadata = {};
      if (next() < 0.3) metadata.required = false;
      if (next() < 0.2) metadata.allowComment = next() < 0.5;
      const earlier = questions.filter((other) => other.type !== 'text');
      const roll = next();
      if (roll < 0.35 && earlier.length > 0) {
        const target = pick(earlier);
        const wanted = target.values.filter(() => next() < 0.5);
        metadata.showWhen = { kind: 'answer', questionId: target.id, values: wanted.length ? wanted : [target.values[0]] };
      } else if (roll < 0.5) {
        metadata.showWhen = { kind: 'scenario', field: pick(FIELDS), value: spaced(next, pick(WORDS)) };
      }
      questions.push({
        id: uuid(next),
        label: `Question ${q + 1}`,
        type,
        // position keeps sequence numbers unique so both sides agree on the order
        sequenceNumber: Math.floor(next() * 100) * 10 + q,
        values,
        valueLabels: next() < 0.5 ? values.map((value) => `Label ${value}`) : [],
        metadata,
        linters: [],
      });
    }
    const scenarioContext = next() < 0.15 ? null : Object.fromEntries(
      FIELDS.filter(() => next() < 0.7).map((field) => [field, next() < 0.1 ? '' : spaced(next, pick(WORDS))]),
    );
    const task = {
      taskId: `task-${index}`,
      format: 'default',
      formDisplayName: 'Parity form',
      formLinters: [],
      scenarioContext,
      subjects: [{ id: `audio-${index}`, label: 'Recording', position: 0, mediaUrl: 'https://example.invalid/clip.wav' }],
      questions,
    };
    const answers = {};
    const comments = {};
    for (const question of questions) {
      if (next() < 0.15) continue;
      if (question.type === 'text') {
        answers[question.id] = [pick(['yes', 'Ответ, с запятой', 'two words'])];
      } else if (question.type === 'multiselect') {
        const chosen = question.values.filter(() => next() < 0.5);
        answers[question.id] = chosen.length ? chosen : [question.values[0]];
      } else {
        answers[question.id] = [pick(question.values)];
      }
      if (commentable(question) && next() < 0.3) comments[question.id] = pick(['  ', ' note ', 'comment', '']);
    }
    for (const question of questions) {
      if (question.metadata.required === false && commentable(question) && !(question.id in answers) && next() < 0.5) {
        comments[question.id] = 'only a comment';
      }
    }
    corpus.push({ id: index, task, answers, comments });
  }
  return corpus;
}
