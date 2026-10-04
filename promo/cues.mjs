// Everything the picture and the soundtrack have to agree on: when each thing
// happens and the words that appear, so a keystroke is heard on the frame it is seen.

// Eight bars at 128 bpm is exactly fifteen seconds, so every cut can sit on a beat.
export const BEAT = 60 / 128;
export const beat = (count) => count * BEAT;
export const DURATION = beat(32);

export const T = {
  agent: beat(1),
  writes: beat(2),
  period: beat(3),
  glass: beat(3) + 0.2,
  zoom: beat(4),
  irisOpen: beat(8) - 0.09,
  land: beat(8),
  steps: [beat(9), beat(9.25), beat(9.5)],
  thread: beat(10),
  typing: [beat(10.4), beat(11.9)],
  send: beat(12),
  strike: beat(12.4),
  rewrite: [beat(13), beat(13.7)],
  fold: beat(13.9),
  staged: beat(14.6),
  collapse: beat(15),
  handoff: beat(15) + 0.28,
  panRight: [beat(15.5), beat(16.12)],
  commit: beat(16),
  rebase: beat(17),
  push: beat(18),
  panUp: [beat(19), beat(20.08)],
  pullRequest: beat(20),
  checks: [beat(21), beat(21.5), beat(22)],
  press: beat(22.5),
  merged: beat(23),
  wipe: beat(23.3),
  blue: beat(24),
  wordmark: beat(25.2),
  tagline: beat(26),
  address: beat(28),
};

export const HEADLINE = ['Agent', 'writes'];
export const COMMENT = 'Flat first, then the percentage.';
export const TITLES = {
  review: ['You', 'review.'],
  ship: ['Ship', 'with Git.'],
  finish: ['Finish in', 'GitHub.'],
};
export const COMMIT_MESSAGE = 'Stack percentage with flat';
export const TAGLINE = 'The review app for code your agent writes.';

// The agent gets the order of operations wrong, and the review comment sends
// it back to rewrite the expression.
export const WRONG = 'subtotal(cart) * (1 - rate) - flat;';
export const FIXED = '(subtotal(cart) - flat) * (1 - rate);';

export function random(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
    mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

const KEYWORDS = new Set(['import', 'export', 'from', 'const', 'let', 'function', 'interface', 'type', 'new']);
const CONTROL = new Set(['return', 'if', 'throw']);
const BUILT_IN = new Set(['number', 'string', 'boolean', 'Record', 'Math', 'Error']);
const TOKEN = /(\s+)|("[^"]*"|`[^`]*`)|(\d[\d.]*)|([A-Za-z_$][\w$]*)|(=>|===|!==|[^\s\w"`])/g;

export function tokenize(line) {
  const tokens = [];
  for (const match of line.matchAll(TOKEN)) {
    const [text, space, string, number, word] = match;
    let kind = 'pun';
    if (space) kind = null;
    else if (string) kind = 'str';
    else if (number) kind = 'num';
    else if (word) {
      const call = line[match.index + text.length] === '(';
      if (KEYWORDS.has(word)) kind = 'kw';
      else if (CONTROL.has(word)) kind = 'ctl';
      else if (BUILT_IN.has(word) || /^[A-Z][a-z]/.test(word)) kind = 'type';
      else if (call) kind = 'fn';
      else kind = 'var';
    }
    tokens.push({ text, kind });
  }
  return tokens;
}

// A person types with an uneven rhythm, one key at a time.
function keystrokes(length, start, end, seed) {
  const next = random(seed);
  return Array.from({ length }, (_, index) => start + ((index + 0.25 + next() * 0.7) / length) * (end - start));
}

// The agent's text arrives in even bursts, a token at a time.
const stream = (count, start, end) => Array.from({ length: count }, (_, index) => start + ((index + 1) / count) * (end - start));

const titleLength = (lines) => lines.join('').length;

export const CUES = {
  headline: HEADLINE.map((word, line) => [...word].map((_, index) => (line === 0 ? T.agent : T.writes) + index * 0.03)),
  // Rows of the reviewed hunk, in the order the agent writes them.
  hunk: Array.from({ length: 10 }, (_, order) => (order < 6 ? 3.27 + order * 0.045 : 3.55 + (order - 6) * 0.05)),
  reviewTitle: keystrokes(titleLength(TITLES.review), T.land + 0.16, T.land + 0.6, 3),
  comment: keystrokes(COMMENT.length, T.typing[0], T.typing[1], 7),
  rewrite: stream(tokenize(FIXED).filter((token) => token.kind).length, T.rewrite[0], T.rewrite[1]),
  shipTitle: stream(titleLength(TITLES.ship), T.commit + 0.04, T.commit + 0.54),
  commitMessage: stream(COMMIT_MESSAGE.length, T.commit + 0.05, T.commit + 0.42),
  finishTitle: stream(titleLength(TITLES.finish), T.pullRequest + 0.04, T.pullRequest + 0.54),
  wordmark: Array.from({ length: 5 }, (_, index) => T.wordmark + 0.1 + index * 0.05),
  tagline: TAGLINE.split(' ').map((_, index) => T.tagline + index * 0.085),
};
