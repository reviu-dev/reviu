// The film is a pure function of time: FILM.seek(t) draws the picture at t and
// nothing animates on its own, which lets the renderer ask for any instant, as
// many times as it likes, and always get the same pixels.
import {
  BEAT,
  COMMENT,
  COMMIT_MESSAGE,
  CUES,
  DURATION,
  FIXED,
  HEADLINE,
  T,
  TAGLINE,
  TITLES,
  WRONG,
  random,
  tokenize,
} from './cues.mjs';

const params = new URLSearchParams(location.search);
const rendering = params.has('render');
const stage = document.getElementById('stage');

// ---------------------------------------------------------------- maths

const clamp = (value, low = 0, high = 1) => Math.min(high, Math.max(low, value));
const lerp = (from, to, amount) => from + (to - from) * amount;
const span = (time, start, end) => clamp((time - start) / (end - start));
const mix = (from, to, amount) => from.map((channel, index) => lerp(channel, to[index], amount));
const toCss = (color) => `rgb(${color.map(Math.round).join(',')})`;
const mixColor = (from, to, amount) => toCss(mix(from, to, amount));

function bezier(x1, y1, x2, y2) {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const curveX = (u) => ((ax * u + bx) * u + cx) * u;
  const curveY = (u) => ((ay * u + by) * u + cy) * u;
  return (amount) => {
    if (amount <= 0) return 0;
    if (amount >= 1) return 1;
    let low = 0;
    let high = 1;
    let u = amount;
    for (let step = 0; step < 24; step++) {
      if (curveX(u) < amount) low = u;
      else high = u;
      u = (low + high) / 2;
    }
    return curveY(u);
  };
}

const ease = {
  outCubic: (p) => 1 - (1 - p) ** 3,
  outExpo: (p) => (p >= 1 ? 1 : (1 - 2 ** (-10 * p)) / (1 - 2 ** -10)),
  inExpo: (p) => (p <= 0 ? 0 : (2 ** (10 * p) - 1) / (2 ** 10 - 1)),
  inOutCubic: (p) => (p < 0.5 ? 4 * p ** 3 : 1 - (-2 * p + 2) ** 3 / 2),
  outBack: (p, overshoot = 1.7) => 1 + (overshoot + 1) * (p - 1) ** 3 + overshoot * (p - 1) ** 2,
  whip: bezier(0.72, 0, 0.16, 1),
  glide: bezier(0.3, 0, 0.1, 1),
};

// A damped spring that overshoots once and comes to rest exactly at 1.
function settle(amount, damping = 6.5, frequency = 7.5) {
  if (amount <= 0) return 0;
  if (amount >= 1) return 1;
  const rest = 1 - Math.exp(-damping) * Math.cos(frequency);
  return (1 - Math.exp(-damping * amount) * Math.cos(frequency * amount)) / rest;
}

const pulse = (time, at, rate) => (time < at ? 0 : Math.exp(-(time - at) * rate));

// ---------------------------------------------------------------- dom

function el(tag, className, parent, html) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (html !== undefined) node.innerHTML = html;
  if (parent) parent.appendChild(node);
  return node;
}

const ICONS = {
  check: '<path d="M20 6 9 17l-5-5"/>',
  circleCheck: '<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>',
  pullRequest:
    '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" x2="6" y1="9" y2="21"/>',
  merge: '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>',
};
const icon = (name, size) =>
  `<svg class="icon" width="${size}" height="${size}" viewBox="0 0 24 24">${ICONS[name]}</svg>`;

// Text that appears as if typed. The whole string is laid out once and then
// uncovered, so kerning never shifts as characters arrive.
function typed(parent, text) {
  const node = el('span', 'typed', parent);
  node.textContent = text;
  return { node, text, widths: [0], shown: -1 };
}

function measureTyped(item) {
  const range = document.createRange();
  const textNode = item.node.firstChild;
  const left = item.node.getBoundingClientRect().left;
  for (let count = 1; count <= item.text.length; count++) {
    range.setStart(textNode, 0);
    range.setEnd(textNode, count);
    item.widths.push(range.getBoundingClientRect().right - left);
  }
  item.width = item.widths[item.text.length];
}

function showTyped(item, count) {
  const visible = item.widths[clamp(count, 0, item.text.length)];
  if (count !== item.shown) {
    item.shown = count;
    item.node.style.clipPath = `inset(-40px ${item.width - visible}px -40px -20px)`;
  }
  return visible;
}

const typedCount = (times, time) => times.reduce((count, at) => count + (time >= at ? 1 : 0), 0);

function blink(time, since) {
  const phase = ((time - since) / BEAT) % 1;
  return phase < 0.55 ? 1 : phase < 0.62 ? 1 - (phase - 0.55) / 0.07 : phase > 0.93 ? (phase - 0.93) / 0.07 : 0;
}

// ---------------------------------------------------------------- code

const highlight = (line) =>
  tokenize(line)
    .map(({ text, kind }) => (kind ? `<span class="t-${kind}">${text.replace(/</g, '&lt;')}</span>` : text))
    .join('');

const CORPUS = [
  'import { type Cart, subtotal } from "./cart";',
  'import { roundCurrency } from "./pricing";',
  'import { taxAmount } from "./tax";',
  '',
  'export interface FlatDiscount {',
  '  kind: "flat";',
  '  amount: number;',
  '}',
  '',
  'export interface PercentDiscount {',
  '  kind: "percent";',
  '  rate: number;',
  '}',
  '',
  'export type Discount = FlatDiscount | PercentDiscount;',
  '',
  'const PROMO_CODES: Record<string, Discount> = {',
  '  LAUNCH10: { kind: "percent", rate: 0.1 },',
  '  WELCOME5: { kind: "flat", amount: 500 },',
  '};',
  '',
  'export function resolvePromo(code: string) {',
  '  const promo = PROMO_CODES[code.toUpperCase()];',
  '  if (!promo) throw new Error(`Unknown code ${code}`);',
  '  return promo;',
  '}',
  '',
  'export function isPercent(discount: Discount) {',
  '  return discount.kind === "percent";',
  '}',
  '',
  'export function clampTotal(value: number) {',
  '  return Math.max(0, value);',
  '}',
  '',
  'describe("stacked discounts", () => {',
  '  it("applies flat before percent", () => {',
  '    const cart = cartWith([desk, lamp]);',
  '    const total = applyDiscounts(cart, [flat, percent]);',
  '    expect(total).toBe(78570);',
  '  });',
  '',
  '  it("never drops below zero", () => {',
  '    const total = applyDiscounts(cart, [oversized]);',
  '    expect(total).toBe(0);',
  '  });',
  '});',
  '',
  'export function cartTotal(cart: Cart, codes: string[]) {',
  '  const discounts = codes.map(resolvePromo);',
  '  const net = applyDiscounts(cart, discounts);',
  '  const tax = taxAmount(net, cart.currency);',
  '  return roundCurrency(net + tax);',
  '}',
  '',
  'export function lineTotals(cart: Cart) {',
  '  return cart.items.map((item) => ({',
  '    sku: item.sku,',
  '    quantity: item.quantity,',
  '    subtotal: item.unitPrice * item.quantity,',
  '  }));',
  '}',
  '',
];

// The hunk the review happens in. The target line is where the agent's wrong
// expression sits until the comment has it rewritten.
const HUNK = [
  ['context', 'export function applyDiscounts('],
  ['context', '  cart: Cart,'],
  ['added', '  discounts: Discount[],'],
  ['context', '): number {'],
  ['added', '  const flat = sumFlat(discounts);'],
  ['added', '  const rate = sumRates(discounts);'],
  ['target', '  return '],
  ['context', '}'],
  ['context', ''],
  ['added', 'function sumRates(discounts: Discount[]) {'],
  ['added', '  return discounts'],
  ['added', '    .filter(isPercent)'],
  ['added', '    .reduce((sum, d) => sum + d.rate, 0);'],
  ['added', '}'],
  ['added', ''],
  ['context', 'function sumFlat(discounts: Discount[]) {'],
  ['context', '  return discounts'],
  ['context', '    .filter(isFlat)'],
  ['context', '    .reduce((sum, d) => sum + d.amount, 0);'],
  ['context', '}'],
];
const TARGET = 6;
const HUNK_ORDER = [9, 10, 11, 12, 13, 14, 2, 4, 5, 6];
const FIRST_LINE = 42;

const COLUMNS = 6;
const HERO = 3;
const ROWS = 76;
const WINDOW = 30;
const ROW = 44;
const COLUMN = 1080;
const CHARACTER = 15.6;
const CODE_LEFT = 116;
const FOCUS = { x: HERO * COLUMN + COLUMN / 2, y: WINDOW * ROW + 392 };
const REST = { x: 740, y: 120, width: COLUMN, height: 840 };
const WALL_FAR = 0.34;
const WALL_NEAR = 1.78;
const THREAD = 196;

// ---------------------------------------------------------------- build

const world = el('div', '', stage);
world.id = 'world';

const backlight = (parent, x, y, width, height, color = '37, 99, 235') => {
  const node = el('div', 'backlight', parent);
  node.style.cssText = `left:${x - width / 2}px;top:${y - height / 2}px;width:${width}px;height:${height}px;
    background:radial-gradient(closest-side, rgba(${color}, 0.26), rgba(${color}, 0.09) 52%, rgba(${color}, 0))`;
  return node;
};
const paneLight = backlight(world, 1280, 540, 1700, 1300);

const pane = el('div', '', world);
pane.id = 'pane';
const wall = el('div', '', pane);
wall.id = 'wall';
wall.style.width = `${COLUMNS * COLUMN}px`;
wall.style.height = `${ROWS * ROW + THREAD}px`;

const rows = [];
const writers = [];
let heroColumn;
let cursorLine;
let thread;
let rewrite;

function buildWall() {
  for (let column = 0; column < COLUMNS; column++) {
    const columnNode = el('div', 'column', wall);
    columnNode.style.left = `${column * COLUMN}px`;
    if (column === HERO) heroColumn = columnNode;

    for (let row = 0; row < ROWS; row++) {
      const inHunk = column === HERO && row >= WINDOW && row < WINDOW + HUNK.length;
      const [kind, text] = inHunk ? HUNK[row - WINDOW] : ['filler', CORPUS[(row + column * 17) % CORPUS.length]];
      const number = column === HERO ? row - WINDOW + FIRST_LINE : row + 1 + column * 31;
      const node = el('div', 'row', columnNode);
      const bar = kind === 'added' || kind === 'target' ? el('span', 'bar', node) : null;
      el('span', 'number', node, String(Math.max(1, number)));
      const code = el('span', 'code', node, highlight(text));

      let appear;
      if (inHunk) {
        const order = HUNK_ORDER.indexOf(row - WINDOW);
        appear = order < 0 ? null : CUES.hunk[order];
      } else {
        appear = 0.3 + (((row + column * 29) % 38) / 38) * 2.5;
      }

      const entry = { node, bar, code, kind, appear, length: text.length, row, column, key: '' };
      rows.push(entry);

      if (kind === 'target') {
        rewrite = el('span', 'rewrite', code);
        rewrite.before = el('span', 'before', rewrite, highlight(WRONG));
        rewrite.after = el('span', 'after', rewrite, highlight(FIXED));
        entry.length = text.length + WRONG.length;
      }

      if (inHunk && row - WINDOW === TARGET) {
        thread = el('div', '', columnNode);
        thread.id = 'thread';
      }
    }

    for (let head = 0; head < 2; head++) writers.push({ node: el('div', 'writer', columnNode), column, head });
  }

  cursorLine = el('div', '', heroColumn);
  cursorLine.id = 'cursorLine';
  writers.push({ node: el('div', 'writer', heroColumn), column: HERO, head: 'hunk' });
}

const header = el(
  'div',
  '',
  pane,
  `<span class="file-type">TS</span><span>src/discounts.ts</span>
   <span id="staged">${icon('circleCheck', 22)}Staged</span>
   <span class="stat">+${HUNK.filter(([kind]) => kind !== 'context').length}</span>`,
);
header.id = 'paneHeader';
const paneEdge = el('div', '', pane);
paneEdge.id = 'paneEdge';

let comment;
function buildComment() {
  comment = el(
    'div',
    '',
    thread,
    `<div class="author"><span class="avatar"></span>You<span class="where">on line ${FIRST_LINE + TARGET}</span></div>
     <div class="body"></div>
     <div id="send"></div>`,
  );
  comment.id = 'comment';
  const body = comment.querySelector('.body');
  comment.text = typed(body, COMMENT);
  comment.caret = el('span', 'caret', body);
  comment.send = comment.querySelector('#send');
}

function buildTitle(parent, lines, detail) {
  const node = el('div', 'title', parent);
  const title = { node, lines: [], total: 0 };
  for (const text of lines) {
    const line = el('div', 'line', node);
    title.lines.push({ text: typed(line, text), caret: el('span', 'caret', line) });
    title.total += text.length;
  }
  title.detail = el('div', 'detail', node, detail);
  return title;
}

const titles = {};
titles.review = buildTitle(world, TITLES.review, 'Inline comments on local diffs');

// Graph: the camera pans right to it, following the commit.
const SHIP = { x: 1920, y: 0 };
const LANE = { main: 900, feature: 988 };
const rowY = (row) => 262 + 130 * row;
const COMMITS = [
  { id: 'tip', lane: 'feature', message: COMMIT_MESSAGE, hashes: ['c4f81d2', 'e71a3c5'], rows: [0, 0], chip: 'feature' },
  { id: 'main', lane: 'main', message: 'Round tax per line item', hashes: ['a1c9e04'], rows: [1, 2], chip: 'main' },
  { id: 'earlier', lane: 'feature', message: 'Add promo code resolver', hashes: ['5e2c7aa', '9d04b6e'], rows: [2, 1] },
  { id: 'older', lane: 'main', message: 'Bump vitest to 3.2', hashes: ['77b0d1f'], rows: [3, 3] },
  { id: 'base', lane: 'main', message: 'Extract cart subtotal', hashes: ['0f3d9b2'], rows: [4, 4] },
];

const ship = el('div', 'scene', world);
ship.style.left = `${SHIP.x}px`;
ship.style.top = `${SHIP.y}px`;
const graphLight = backlight(ship, LANE.feature + 230, 430, 1500, 1200);
titles.ship = buildTitle(
  ship,
  TITLES.ship,
  ['Stage', 'commit', 'rebase', 'push'].map((step) => `<span class="step">${step}</span>`).join(', '),
);
const steps = [...titles.ship.detail.querySelectorAll('.step')];

const graph = el('div', '', ship);
graph.innerHTML = `
  <svg id="graph" width="1920" height="1080" viewBox="0 0 1920 1080">
    <defs>
      <linearGradient id="trail" gradientUnits="userSpaceOnUse" x1="0" y1="760" x2="0" y2="1040">
        <stop offset="0" stop-color="#8f8f8f"/><stop offset="1" stop-color="#8f8f8f" stop-opacity="0"/>
      </linearGradient>
    </defs>
    <path id="mainLine" fill="none" stroke="url(#trail)" stroke-width="3"/>
    <path id="featureLine" fill="none" stroke="#2563eb" stroke-width="3" stroke-linecap="round"/>
    <circle id="ring" fill="none" stroke="#2563eb"/>
    <g id="nodes"></g>
  </svg>`;
const mainLine = graph.querySelector('#mainLine');
const featureLine = graph.querySelector('#featureLine');
const ring = graph.querySelector('#ring');
const nodes = graph.querySelector('#nodes');

for (const commit of COMMITS) {
  commit.dot = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
  if (commit.lane === 'feature') commit.dot.setAttribute('fill', '#2563eb');
  else {
    commit.dot.setAttribute('fill', '#000000');
    commit.dot.setAttribute('stroke', '#8f8f8f');
    commit.dot.setAttribute('stroke-width', '3');
  }
  nodes.appendChild(commit.dot);

  commit.node = el('div', 'commit', ship);
  const message = el('div', 'message', commit.node);
  commit.text = typed(message, commit.message);
  commit.meta = el('div', 'meta', commit.node);
  commit.hash = el('span', '', commit.meta, commit.hashes[0]);
  if (commit.chip === 'main') el('span', 'chip', commit.meta, 'main');
  if (commit.chip === 'feature') el('span', 'chip feature', commit.meta, 'feature/checkout-discounts');
  commit.shown = '';
}
const commitById = Object.fromEntries(COMMITS.map((commit) => [commit.id, commit]));

// Pull request: the camera tilts up to it, following the push.
const FINISH = { x: 1920, y: -1080 };
const tether = el('div', '', world);
tether.innerHTML =
  '<svg id="tether" width="1" height="1"><path fill="none" stroke="#2563eb" stroke-width="3" stroke-linecap="round"/></svg>';
const tetherPath = tether.querySelector('path');
const finish = el('div', 'scene', world);
finish.style.left = `${FINISH.x}px`;
finish.style.top = `${FINISH.y}px`;
const cardLight = backlight(finish, 1290, 532, 1700, 1300);
const mergeLight = backlight(finish, 1290, 532, 1700, 1300, '167, 139, 250');
titles.finish = buildTitle(finish, TITLES.finish, 'PR, checks, review, merge<span class="tag">Reviu Pro</span>');

const CHECKS = [
  ['CI / Typecheck', 'Successful in 12s'],
  ['CI / Tests', 'Successful in 41s'],
  ['Noah Fischer', 'Approved'],
];
const pullRequest = el(
  'div',
  '',
  finish,
  `<div class="head">
     <div class="status">
       <span id="pill"><span class="glyph"></span><span class="label"></span></span>
       <span class="pr-number">#128</span>
     </div>
     <div class="pr-title">Stack percentage discounts with flat</div>
     <div class="branches">feature/checkout-discounts into main</div>
   </div>
   ${CHECKS.map(
     ([name, outcome]) => `
   <div class="check">
     <span class="mark"><span class="spinner"></span><span class="passed">${icon('circleCheck', 30)}</span></span>
     <span>${name}</span><span class="outcome">${outcome}</span>
   </div>`,
   ).join('')}
   <div class="foot"><div id="merge"><span class="label"></span><div id="mergeRing"></div></div></div>`,
);
pullRequest.id = 'pullRequest';
const pill = pullRequest.querySelector('#pill');
const merge = pullRequest.querySelector('#merge');
const mergeRing = pullRequest.querySelector('#mergeRing');
const checks = [...pullRequest.querySelectorAll('.check')].map((node) => ({
  spinner: node.querySelector('.spinner'),
  passed: node.querySelector('.passed'),
  outcome: node.querySelector('.outcome'),
}));

const dot = el('div', '', world);
dot.id = 'dot';

// Opening: the headline is cut out of a black sheet so the code shows through it.
const mask = el('canvas', '', stage);
mask.id = 'mask';
const ratio = window.devicePixelRatio || 1;
mask.width = 1920 * ratio;
mask.height = 1080 * ratio;
const ink = mask.getContext('2d');
const openingDetail = el('div', '', stage, 'Claude Code, Codex, Gemini');
openingDetail.id = 'openingDetail';

const HEAD = { size: 300, left: 120, baselines: [444, 756] };
const headFont = `600 ${HEAD.size}px Poppins`;
const headTracking = `${-0.04 * HEAD.size}px`;
const opening = { letters: [], caret: [], period: null, zoom: 1 };

function buildOpening() {
  ink.font = headFont;
  ink.letterSpacing = headTracking;
  const widthOf = (text) => ink.measureText(text).width;
  const gap = 0.045 * HEAD.size - parseFloat(headTracking);

  opening.caret.push({ time: -1, x: HEAD.left, line: 0 });
  HEADLINE.forEach((text, line) => {
    if (line === 1) opening.caret.push({ time: T.writes - 0.13, x: HEAD.left, line });
    [...text].forEach((character, index) => {
      const end = HEAD.left + widthOf(text.slice(0, index + 1));
      const appear = CUES.headline[line][index];
      opening.letters.push({ character, line, x: end - widthOf(character), end, appear });
      opening.caret.push({ time: appear, x: end + gap, line });
    });
  });

  const last = opening.letters.at(-1);
  const box = ink.measureText('.');
  const radius = (box.actualBoundingBoxRight + box.actualBoundingBoxLeft) / 2;
  opening.period = {
    x: last.end + (box.actualBoundingBoxRight - box.actualBoundingBoxLeft) / 2,
    y: HEAD.baselines[1] + (box.actualBoundingBoxDescent - box.actualBoundingBoxAscent) / 2,
    radius,
  };
  opening.caret.push({ time: T.period, x: last.end + widthOf('.') + gap, line: 1 });
  // The period opens like an iris until it clears the far corner of the frame.
  opening.zoom = (Math.hypot(960, 540) * 1.1) / radius;
}

// Ending
const ending = el('div', 'scene', stage);
ending.style.transformOrigin = '960px 500px';
const glow = el('div', '', ending);
glow.id = 'glow';
const MARK = 'M225.5,272.1c52.7-20.4,90-71.5,90-131.3S252.5,0,174.8,0H0v361.1h315.6l-90-89Z';
const LETTERS = [
  'M487.8,284.6h-56.2v76.5h-49.3V120h118.9c45.1,0,82,37.2,82,82.3s-17.2,57.9-43.1,72c19.6,28.9,38.9,57.9,58.2,86.8h-59.3l-51.3-76.5ZM431.6,235.4h69.6c17.9-.3,32.7-15.2,32.7-33.1s-15.2-33.1-33.1-33.1h-69.3v66.2Z',
  'M664.9,169.2v46.9h115.1v49.3h-115.1v46.5h132v49.3h-181.2V120h181.2v49.3h-132Z',
  'M1016.6,120h53.7c-35.8,80.3-71.3,160.9-107.2,241.2h-42l-107.2-241.2h53.7c24.8,55.8,49.6,111.6,74.4,167.4,24.8-55.8,49.6-111.6,74.4-167.4Z',
  'M1087.6,361.1V120h49.3v241.2h-49.3Z',
  'M1210.3,260.9c0,27.9,23.1,51.3,51.3,51,27.9,0,51-23.1,51-51V120h49.3v140.9c0,55.1-44.8,100.3-99.9,100.3s-100.9-44.8-100.9-100.3V120h49.3v140.9Z',
];
const logoHolder = el('div', '', ending);
logoHolder.innerHTML = `
  <svg id="logo" width="1920" height="1080" viewBox="0 0 1920 1080">
    <defs><clipPath id="behindMark"><rect id="reveal" y="0" height="1080" width="1920"/></clipPath></defs>
    <g clip-path="url(#behindMark)">${LETTERS.map((path) => `<path class="letter" fill="#ffffff" d="${path}"/>`).join('')}</g>
    <path id="mark" fill="#2563eb" d="${MARK}"/>
  </svg>`;
const markPath = logoHolder.querySelector('#mark');
const reveal = logoHolder.querySelector('#reveal');
const letterPaths = [...logoHolder.querySelectorAll('.letter')];

const tagline = el('div', '', ending, '<span class="holder"></span>');
tagline.id = 'tagline';
const taglineHolder = tagline.querySelector('.holder');
tagline.text = typed(taglineHolder, TAGLINE);
tagline.caret = el('span', 'caret', taglineHolder);
// The agent's text arrives a token at a time, so the tagline does too.
tagline.tokens = [...TAGLINE.matchAll(/\S+\s*/g)].map((match) => match.index + match[0].trimEnd().length);
const address = el('div', '', ending, 'reviu.dev');
address.id = 'address';

const wipe = el('div', '', stage);
wipe.id = 'wipe';

const timecode = el('div', '', stage);
timecode.id = 'timecode';
const probe = el('div', '', stage);
probe.id = 'probe';

// ---------------------------------------------------------------- camera

function camera(time) {
  const right = ease.whip(span(time, ...T.panRight));
  const up = ease.whip(span(time, ...T.panUp));
  const push =
    0.03 * span(time, T.land + 0.7, T.collapse) * (1 - right) +
    0.024 * span(time, T.commit, T.panUp[0]) * right * (1 - up) +
    0.024 * span(time, T.pullRequest, T.blue) * up;
  const kick = 0.012 * pulse(time, T.commit, 9) + 0.012 * pulse(time, T.pullRequest, 9) + 0.016 * pulse(time, T.merged, 9);
  return { x: SHIP.x * right, y: FINISH.y * up, zoom: 1 + push + kick, right, up };
}

const toScreen = (view, x, y) => ({
  x: (x - view.x - 960) * view.zoom + 960,
  y: (y - view.y - 540) * view.zoom + 540,
});
const toWorld = (view, x, y) => ({
  x: (x - 960) / view.zoom + 960 + view.x,
  y: (y - 540) / view.zoom + 540 + view.y,
});

// ---------------------------------------------------------------- opening

function caretAt(time) {
  let index = 0;
  while (index + 1 < opening.caret.length && opening.caret[index + 1].time <= time) index++;
  const to = opening.caret[index];
  const from = opening.caret[Math.max(0, index - 1)];
  const amount = ease.outExpo(span(time, to.time, to.time + 0.09));
  return {
    x: lerp(from.x, to.x, amount),
    baseline: lerp(HEAD.baselines[from.line], HEAD.baselines[to.line], amount),
  };
}

function drawOpening(time) {
  const live = time < T.irisOpen;
  mask.style.display = live ? 'block' : 'none';
  openingDetail.style.display = live ? 'block' : 'none';
  if (!live) return;

  const { period } = opening;
  const glass = ease.inOutCubic(span(time, T.glass, T.glass + 0.42));
  const zoom = opening.zoom ** (span(time, T.zoom, T.irisOpen) ** 2.4);
  const drift = ease.inOutCubic(span(time, T.zoom, T.zoom + 1.45));
  const centre = { x: lerp(period.x, 960, drift), y: lerp(period.y, 540, drift) };
  const onScreen = (left, right, top, bottom) =>
    centre.x + (right - period.x) * zoom > 0 &&
    centre.x + (left - period.x) * zoom < 1920 &&
    centre.y + (bottom - period.y) * zoom > 0 &&
    centre.y + (top - period.y) * zoom < 1080;

  ink.setTransform(ratio, 0, 0, ratio, 0, 0);
  ink.globalCompositeOperation = 'source-over';
  ink.globalAlpha = 1;
  ink.shadowBlur = 0;
  ink.fillStyle = '#000000';
  ink.fillRect(0, 0, 1920, 1080);
  ink.translate(centre.x, centre.y);
  ink.scale(zoom, zoom);
  ink.translate(-period.x, -period.y);
  ink.font = headFont;
  ink.letterSpacing = headTracking;
  ink.textBaseline = 'alphabetic';
  ink.textAlign = 'left';

  const periodScale = ease.outBack(span(time, T.period, T.period + 0.3), 2.6);
  const visible = opening.letters.filter(
    (letter) =>
      time > letter.appear &&
      onScreen(letter.x, letter.end, HEAD.baselines[letter.line] - HEAD.size, HEAD.baselines[letter.line] + HEAD.size * 0.3),
  );

  const drawPeriod = () => {
    if (time <= T.period) return;
    ink.beginPath();
    ink.arc(period.x, period.y, period.radius * periodScale, 0, Math.PI * 2);
    ink.fill();
  };

  if (glass > 0) {
    ink.globalCompositeOperation = 'destination-out';
    for (const letter of visible) ink.fillText(letter.character, letter.x, HEAD.baselines[letter.line]);
    drawPeriod();
    ink.globalCompositeOperation = 'source-over';
  }

  if (glass < 1) {
    ink.fillStyle = '#fafafa';
    for (const letter of visible) {
      const rise = (1 - ease.outExpo(span(time, letter.appear, letter.appear + 0.45))) * HEAD.size * 0.26;
      ink.globalAlpha = (1 - glass) * span(time, letter.appear, letter.appear + 0.08);
      ink.fillText(letter.character, letter.x, HEAD.baselines[letter.line] + rise);
    }
    ink.globalAlpha = 1 - glass;
    drawPeriod();
  }

  const outline = 0.4 * glass * (1 - span(time, T.zoom + 0.5, T.zoom + 1.3));
  if (outline > 0) {
    ink.globalAlpha = outline;
    ink.strokeStyle = '#ffffff';
    ink.lineWidth = 2 / zoom;
    for (const letter of visible) ink.strokeText(letter.character, letter.x, HEAD.baselines[letter.line]);
    ink.beginPath();
    ink.arc(period.x, period.y, period.radius, 0, Math.PI * 2);
    ink.stroke();
  }

  const caret = caretAt(time);
  const idle = time < T.agent ? blink(time, -0.07) : time > T.period + 0.1 && time < T.zoom ? blink(time, T.period + 0.1 - BEAT * 0.4) : 1;
  const caretTop = caret.baseline - HEAD.size * 0.8;
  if (onScreen(caret.x, caret.x + 14, caretTop, caret.baseline + HEAD.size * 0.1)) {
    ink.globalAlpha = idle;
    ink.fillStyle = '#2563eb';
    ink.shadowColor = `rgba(37, 99, 235, ${0.75 + 0.25 * pulse(time, 0, 4)})`;
    ink.shadowBlur = (30 + 50 * pulse(time, 0, 4)) * ratio * Math.min(zoom, 4);
    ink.beginPath();
    ink.roundRect(caret.x, caretTop, HEAD.size * 0.045, HEAD.size * 0.9, 3);
    // Filled twice so the glow, which canvas draws as a shadow, is strong enough to read.
    ink.fill();
    ink.fill();
    ink.shadowBlur = 0;
  }
  ink.globalAlpha = 1;

  const detailIn = span(time, T.agent + 0.3, T.agent + 0.7);
  const detailOut = span(time, T.zoom, T.zoom + 0.3);
  openingDetail.style.opacity = ease.outCubic(detailIn) * (1 - detailOut);
  openingDetail.style.transform = `translateY(${(1 - ease.outExpo(detailIn)) * 16 + detailOut * 30}px)`;
}

// ---------------------------------------------------------------- diff

const GLASS = [27, 100, 27];
const DEEP = [20, 42, 20];
const BLACK = [0, 0, 0];
const BLUE = [37, 99, 235];
const ADDED = [22, 45, 22];
const FRESH = [38, 130, 44];

function updateRows(time) {
  const staging = pulse(time, T.staged, 6);
  for (const row of rows) {
    if (row.appear === null) continue;
    const written = span(time, row.appear, row.appear + 0.13);
    const flash = Math.max(pulse(time, row.appear, 5), row.bar ? staging : 0);
    const key = `${written.toFixed(3)}|${flash.toFixed(2)}`;
    if (key === row.key) continue;
    row.key = key;
    row.code.style.clipPath = written >= 1 ? 'none' : `inset(0 ${(1 - written) * 100}% 0 0)`;
    if (row.bar) {
      row.bar.style.opacity = written > 0 ? 1 : 0;
      row.node.style.background = written > 0 ? mixColor(ADDED, FRESH, flash) : 'transparent';
    } else {
      row.node.style.background = flash > 0.01 ? `rgba(74, 222, 128, ${(flash * 0.42).toFixed(3)})` : 'transparent';
    }
  }

  for (const writer of writers) {
    let row = null;
    if (writer.head === 'hunk') {
      row = rows.find((entry) => entry.column === HERO && entry.bar && time >= entry.appear && time < entry.appear + 0.13);
    } else if (time >= 0.3 && time < 2.8) {
      const slot = Math.floor(((time - 0.3) / 2.5) * 38);
      const index = (((slot - writer.column * 29) % 38) + 38) % 38 + writer.head * 38;
      row = rows[writer.column * ROWS + index];
      if (row.appear === null || row.kind !== 'filler') row = null;
    }
    writer.node.style.display = row ? 'block' : 'none';
    if (!row) continue;
    const written = span(time, row.appear, row.appear + 0.13);
    writer.node.style.left = `${CODE_LEFT + row.length * CHARACTER * written}px`;
    writer.node.style.top = `${row.row * ROW + (row.column === HERO && row.row > WINDOW + TARGET ? threadHeight(time) : 0)}px`;
  }
}

function threadHeight(time) {
  const open = settle(span(time, T.thread, T.thread + 0.55));
  const close = ease.inOutCubic(span(time, T.fold, T.fold + 0.36));
  return THREAD * open * (1 - close);
}

function updatePane(time, view) {
  const live = time < T.handoff;
  pane.style.display = live ? 'block' : 'none';
  paneLight.style.display = live && time >= T.land ? 'block' : 'none';
  if (!live) return;

  const snap = settle(span(time, T.land, T.land + 0.75));
  const collapse = ease.inOutCubic(span(time, T.collapse, T.handoff));
  const width = lerp(lerp(1920, REST.width, snap), 26, collapse);
  const height = lerp(lerp(1080, REST.height, snap), 26, collapse);
  const centreX = lerp(960, REST.x + REST.width / 2, snap);
  const centreY = lerp(540, REST.y + REST.height / 2, snap);
  const settled = clamp(snap);

  pane.style.left = `${centreX - width / 2}px`;
  pane.style.top = `${centreY - height / 2}px`;
  pane.style.width = `${width}px`;
  pane.style.height = `${height}px`;
  pane.style.borderRadius = `${16 * settled + collapse * Math.min(width, height)}px`;
  pane.style.boxShadow = `0 0 ${60 * collapse}px rgba(37, 99, 235, ${0.7 * collapse})`;
  paneEdge.style.borderRadius = pane.style.borderRadius;
  paneEdge.style.opacity = settled * (1 - span(collapse, 0, 0.4));

  const tint = mix(mix(GLASS, DEEP, span(time, 2.0, 2.9)), BLACK, span(time, T.land - 0.3, T.land + 0.25));
  pane.style.background = toCss(mix(tint, BLUE, collapse));

  const zoomIn = span(time, T.zoom, T.land) ** 1.7;
  const scale =
    (time < T.land ? WALL_FAR * (WALL_NEAR / WALL_FAR) ** zoomIn : WALL_NEAR * (1 / WALL_NEAR) ** snap) *
    lerp(1, 0.3, collapse);
  const anchorY = height / 2 + 28 * settled * (1 - collapse);
  const wallTop = anchorY - scale * FOCUS.y;
  wall.style.transform = `translate(${width / 2 - scale * FOCUS.x}px, ${wallTop}px) scale(${scale})`;
  wall.style.opacity = 1 - span(collapse, 0, 0.55);
  // Keeps code out from under the header, where it would bleed through the
  // antialiased top edge of the pane.
  wall.style.clipPath = settled > 0 ? `inset(${(56 * settled - 1 - wallTop) / scale}px 0 0 0)` : 'none';

  paneLight.style.opacity = span(time, T.land + 0.1, T.land + 0.9) * (1 - collapse);
  header.style.opacity = settled * (1 - span(collapse, 0, 0.4));
  header.style.transform = `translateY(${-56 * (1 - settled)}px)`;

  updateRows(time);
  updateReview(time);
}

function updateReview(time) {
  const [first, second, third] = T.steps;
  const line = 4 + ease.outExpo(span(time, second, second + 0.08)) + ease.outExpo(span(time, third, third + 0.08));
  cursorLine.style.top = `${(WINDOW + line) * ROW}px`;
  cursorLine.style.opacity = span(time, first - 0.02, first + 0.05) * (1 - span(time, T.fold, T.fold + 0.3));

  const open = settle(span(time, T.thread, T.thread + 0.55));
  const close = ease.inOutCubic(span(time, T.fold, T.fold + 0.36));
  thread.style.height = `${threadHeight(time)}px`;
  comment.style.opacity = span(time, T.thread + 0.02, T.thread + 0.18) * (1 - span(close, 0, 0.6));
  comment.style.transform = `translateY(${(1 - open) * -16}px) scale(${lerp(0.96, 1, clamp(open))})`;
  comment.style.borderColor = mixColor([38, 38, 38], BLUE, pulse(time, T.send, 5));

  const count = typedCount(CUES.comment, time);
  const width = showTyped(comment.text, count);
  comment.caret.style.left = `${width + 3}px`;
  comment.caret.style.opacity = time < T.send ? (count === 0 ? blink(time, T.thread) : 1) : 0;

  const sent = time >= T.send + 0.07;
  const press = span(time, T.send - 0.06, T.send) * (1 - ease.outCubic(span(time, T.send, T.send + 0.2)));
  const state = sent ? 'sent' : 'ready';
  if (comment.send.dataset.state !== state) {
    comment.send.dataset.state = state;
    comment.send.innerHTML = sent
      ? `${icon('check', 22)}Sent to agent`
      : 'Send to agent<span class="keys">⌘↵</span>';
    comment.send.style.background = sent ? 'transparent' : '#fafafa';
    comment.send.style.color = sent ? '#4ade80' : '#0a0a0a';
  }
  comment.send.style.transform = `scale(${1 - 0.07 * press})`;

  // The agent's answer: the wrong expression is struck, removed, and rewritten.
  const strike = ease.outCubic(span(time, T.strike, T.strike + 0.14));
  const removed = ease.inOutCubic(span(time, T.strike + 0.17, T.rewrite[0] - 0.03));
  rewrite.before.style.background = `linear-gradient(90deg, #7f2323 ${strike * 100}%, transparent ${strike * 100}%)`;
  rewrite.before.style.clipPath = `inset(0 ${removed * 100}% 0 0)`;

  const arrived = typedCount(CUES.rewrite, time);
  const visible = arrived === 0 ? 0 : rewrite.tokens[arrived - 1];
  rewrite.after.style.clipPath = `inset(0 ${rewrite.width - visible}px 0 0)`;
  rewrite.after.style.background = `rgba(27, 100, 27, ${1 - span(time, T.rewrite[1] + 0.1, T.rewrite[1] + 0.55)})`;

  const staged = settle(span(time, T.staged, T.staged + 0.4));
  header.querySelector('#staged').style.transform = `scale(${staged})`;
  header.querySelector('#staged').style.opacity = clamp(staged * 3);
}

// ---------------------------------------------------------------- titles

function updateTitle(title, time, times) {
  const start = times[0];
  const length = times.at(-1) - start;
  const count = typedCount(times, time);
  let before = 0;
  let active = title.lines.length - 1;
  title.lines.forEach((line, index) => {
    const shown = clamp(count - before, 0, line.text.text.length);
    line.width = showTyped(line.text, shown);
    if (active === title.lines.length - 1 && shown < line.text.text.length) active = index;
    before += line.text.text.length;
  });
  const done = count >= title.total;
  const waiting = time < start - 0.04;
  title.lines.forEach((line, index) => {
    line.caret.style.left = `${line.width + 12}px`;
    line.caret.style.opacity = index !== active || waiting ? 0 : done ? blink(time, start + length) : 1;
  });
  const detail = span(time, start + length * 0.7, start + length + 0.3);
  title.detail.style.opacity = ease.outCubic(detail);
  title.detail.style.transform = `translateY(${(1 - ease.outExpo(detail)) * 14}px)`;
}

// ---------------------------------------------------------------- graph

const HEX = '0123456789abcdef';
function scramble(hash, time) {
  const next = random(Math.floor(time * 30) * 977 + hash.charCodeAt(0));
  return [...hash].map(() => HEX[Math.floor(next() * 16)]).join('');
}

function updateGraph(time) {
  const live = time >= T.panRight[0] - 0.05 && time < T.pullRequest + 0.25;
  ship.style.display = live ? 'block' : 'none';
  if (!live) return;

  updateTitle(titles.ship, time, CUES.shipTitle);
  const beats = [T.panRight[0], T.commit, T.rebase, T.push, T.panUp[1]];
  steps.forEach((step, index) => {
    const lit = span(time, beats[index], beats[index] + 0.08) * (1 - 0.45 * span(time, beats[index + 1], beats[index + 1] + 0.2));
    step.style.color = mixColor([92, 92, 92], [250, 250, 250], lit);
  });

  graphLight.style.opacity = span(time, T.commit, T.commit + 0.7) * (0.8 + 0.5 * pulse(time, T.commit, 3));
  const rebase = ease.glide(span(time, T.rebase, T.rebase + 0.46));
  const landed = time >= T.commit;
  const grown = ease.outExpo(span(time, T.commit, T.commit + 0.3));
  const pushed = pulse(time, T.push, 7);
  const position = (commit) => ({
    x: LANE[commit.lane] + (commit.id === 'earlier' ? 40 * Math.sin(Math.PI * rebase) : 0),
    y: rowY(lerp(commit.rows[0], commit.rows[1], rebase)),
  });

  for (const commit of COMMITS) {
    const { x, y } = position(commit);
    const isTip = commit.id === 'tip';
    const born = isTip ? settle(span(time, T.commit, T.commit + 0.45), 5, 9) : 1;
    const radius = 11 * (isTip ? (landed ? lerp(1.5, 1, born) : 0) : 1) * (commit.lane === 'feature' ? 1 + 0.55 * pushed : 1);
    commit.dot.setAttribute('cx', x);
    commit.dot.setAttribute('cy', y);
    commit.dot.setAttribute('r', Math.max(0, radius));

    commit.node.style.top = `${y - 21}px`;
    commit.node.style.transform = `translateX(${x - LANE[commit.lane]}px)`;
    if (commit.id === 'main') commit.node.style.opacity = 1 - 0.8 * Math.sin(Math.PI * rebase) ** 0.7;
    if (isTip) {
      showTyped(commit.text, typedCount(CUES.commitMessage, time));
      commit.meta.style.opacity = span(time, T.commit + 0.3, T.commit + 0.5);
    }

    let hash = commit.hashes[0];
    if (commit.hashes.length > 1 && time >= T.rebase + 0.04) {
      hash = time < T.rebase + 0.42 ? scramble(commit.hashes[1], time) : commit.hashes[1];
    }
    if (hash !== commit.shown) {
      commit.shown = hash;
      commit.hash.textContent = hash;
    }
    commit.hash.style.color = mixColor([126, 127, 133], [157, 187, 251], commit.hashes.length > 1 ? pulse(time, T.rebase + 0.04, 2.2) : 0);
  }

  const tip = position(commitById.tip);
  const earlier = position(commitById.earlier);
  const forkY = lerp(rowY(4), rowY(2), rebase);
  const top = lerp(earlier.y, tip.y, grown);
  featureLine.setAttribute(
    'd',
    `M${LANE.feature},${top} L${earlier.x},${earlier.y} ` +
      `C${earlier.x},${earlier.y + 62} ${LANE.main},${forkY - 62} ${LANE.main},${forkY}`,
  );
  mainLine.setAttribute('d', `M${LANE.main},${position(commitById.main).y} L${LANE.main},1040`);

  const burst = span(time, T.commit, T.commit + 0.5);
  ring.setAttribute('cx', tip.x);
  ring.setAttribute('cy', tip.y);
  ring.setAttribute('r', lerp(12, 74, ease.outExpo(burst)));
  ring.setAttribute('stroke-width', 3 * (1 - burst));
  ring.setAttribute('opacity', landed ? 1 - burst : 0);
}

// ---------------------------------------------------------------- pull request

const PILL = { open: [74, 222, 128], merged: [167, 139, 250] };
let pillState = '';

function updatePullRequest(time) {
  const live = time >= T.panUp[0] - 0.05 && time < T.blue + 0.05;
  finish.style.display = live ? 'block' : 'none';
  if (!live) return;

  updateTitle(titles.finish, time, CUES.finishTitle);

  const born = settle(span(time, T.pullRequest - 0.15, T.pullRequest + 0.42), 6, 8);
  pullRequest.style.transformOrigin = `${pillTarget.x - FINISH.x - 760}px ${pillTarget.y - FINISH.y - 250}px`;
  pullRequest.style.transform = `scale(${lerp(0.2, 1, born)})`;
  pullRequest.style.opacity = span(time, T.pullRequest - 0.15, T.pullRequest - 0.02);

  checks.forEach((check, index) => {
    const passed = settle(span(time, T.checks[index], T.checks[index] + 0.34), 6, 9);
    check.spinner.style.transform = `rotate(${time * 620 + index * 90}deg)`;
    check.spinner.style.opacity = 1 - span(time, T.checks[index] - 0.02, T.checks[index] + 0.05);
    check.passed.style.transform = `scale(${passed})`;
    check.passed.style.opacity = clamp(passed * 4);
    check.outcome.style.opacity = span(time, T.checks[index], T.checks[index] + 0.18);
    check.outcome.style.transform = `translateX(${(1 - ease.outExpo(span(time, T.checks[index], T.checks[index] + 0.3))) * 14}px)`;
  });

  const merged = time >= T.merged;
  const state = merged ? 'merged' : 'open';
  if (state !== pillState) {
    pillState = state;
    const [red, green, blue] = PILL[state];
    pill.querySelector('.glyph').innerHTML = icon(merged ? 'merge' : 'pullRequest', 22);
    pill.querySelector('.glyph').style.display = 'flex';
    pill.querySelector('.label').textContent = merged ? 'Merged' : 'Open';
    pill.style.color = `rgb(${red}, ${green}, ${blue})`;
    pill.style.borderColor = `rgba(${red}, ${green}, ${blue}, 0.4)`;
    pill.style.background = `rgba(${red}, ${green}, ${blue}, 0.13)`;
    merge.querySelector('.label').innerHTML = merged ? `${icon('merge', 24)}Merged` : 'Create a merge commit';
    merge.querySelector('.label').style.cssText = 'display:flex;align-items:center;gap:10px';
  }
  const violet = span(time, T.merged, T.merged + 0.12);
  cardLight.style.opacity = span(time, T.pullRequest, T.pullRequest + 0.7) * (1 - violet);
  mergeLight.style.opacity = violet * (1 + 0.9 * pulse(time, T.merged, 3));
  const flip = pulse(time, T.merged, 7);
  pill.style.transform = `scale(${1 + 0.22 * flip})`;
  const press = span(time, T.press, T.press + 0.07) * (1 - ease.outCubic(span(time, T.merged - 0.05, T.merged + 0.2)));
  merge.style.transform = `scale(${1 - 0.06 * press + 0.08 * flip})`;
  merge.style.background = merged ? '#a78bfa' : mixColor([250, 250, 250], [190, 190, 196], press);
  merge.style.boxShadow = merged ? `0 0 ${50 * flip + 24}px rgba(167, 139, 250, ${0.25 + 0.5 * flip})` : 'none';
  pullRequest.style.borderColor = mixColor([38, 38, 38], PILL.merged, pulse(time, T.merged, 4) * 0.9);

  const spread = ease.outExpo(span(time, T.merged, T.merged + 0.55));
  const reach = 90 * spread;
  mergeRing.style.inset = `${-reach}px`;
  mergeRing.style.borderRadius = `${11 + reach}px`;
  mergeRing.style.opacity = merged ? 0.9 * (1 - spread) : 0;
}

// ---------------------------------------------------------------- travelling commit

let pillTarget = { x: 0, y: 0 };
const TIP = { x: SHIP.x + LANE.feature, y: rowY(0) };
const panRightAt = (time) => ease.whip(span(time, ...T.panRight));

const along = (curve, amount) => {
  const first = curve.slice(0, 3).map((point, index) => point.map((value, axis) => lerp(value, curve[index + 1][axis], amount)));
  const second = first.slice(0, 2).map((point, index) => point.map((value, axis) => lerp(value, first[index + 1][axis], amount)));
  const end = second[0].map((value, axis) => lerp(value, second[1][axis], amount));
  return [curve[0], first[0], second[0], end];
};

function updateDot(time, view) {
  const carrying = time >= T.handoff && time < T.commit;
  const pushing = time >= T.push && time < T.pullRequest + 0.16;
  dot.style.display = carrying || pushing ? 'block' : 'none';
  tether.style.display = time >= T.push && time < T.blue ? 'block' : 'none';

  let at;
  let size = 26;
  if (carrying) {
    // The commit keeps pace with the camera, so it holds still on screen
    // while the world whips past behind it.
    const travel = clamp((view.right - panRightAt(T.handoff)) / (panRightAt(T.commit) - panRightAt(T.handoff)));
    at = {
      x: lerp(REST.x + REST.width / 2, TIP.x, travel),
      y: lerp(REST.y + REST.height / 2, TIP.y, travel),
    };
    size = lerp(26, 33, travel);
  } else if (pushing) {
    const curve = [
      [TIP.x, TIP.y],
      [TIP.x, TIP.y - 430],
      [pillTarget.x, pillTarget.y + 430],
      [pillTarget.x, pillTarget.y],
    ];
    // Where the push should sit in the frame: it leads the camera out of the
    // graph, then the camera catches it up and sets it down on the pull request.
    const dip = Math.sin(Math.PI * span(time, T.push, T.push + 0.14)) * 12;
    const lead = 150 * ease.outCubic(span(time, T.push + 0.07, T.push + 0.5));
    const home = ease.inOutCubic(span(time, T.panUp[0] + 0.04, T.pullRequest));
    const from = toScreen(view, TIP.x, TIP.y);
    const to = toScreen(view, pillTarget.x, pillTarget.y);
    const height = toWorld(view, 0, lerp(from.y + dip - lead, to.y, home)).y;

    let low = 0;
    let high = 1;
    for (let step = 0; step < 24; step++) {
      const middle = (low + high) / 2;
      if (along(curve, middle)[3][1] > height) low = middle;
      else high = middle;
    }
    const drawn = along(curve, low);
    at = height > TIP.y ? { x: TIP.x, y: height } : { x: drawn[3][0], y: drawn[3][1] };
    tetherPath.setAttribute('d', `M${drawn[0]} C${drawn[1]} ${drawn[2]} ${drawn[3]}`);
    tetherPath.setAttribute('opacity', 1 - span(time, T.pullRequest + 0.1, T.pullRequest + 0.6));
    size = lerp(22, 16, home) * (1 - span(time, T.pullRequest - 0.02, T.pullRequest + 0.12));
  } else {
    return;
  }

  dot.style.left = `${at.x - size / 2}px`;
  dot.style.top = `${at.y - size / 2}px`;
  dot.style.width = `${size}px`;
  dot.style.height = `${size}px`;
}

// ---------------------------------------------------------------- ending

const SOLID = { x: 145, y: 141, scale: 7.4 };
const ALONE = { x: 960, y: 500, scale: 330 / 361.1 };
const LOCKUP = { left: 470, top: 290, scale: 980 / 1361.9 };
const MARK_CENTRE = { x: 157.8, y: 180.55 };
let wipeFrom = { x: 0, y: 0, width: 8, height: 104 };

function updateEnding(time, view) {
  const wiping = time >= T.wipe && time < T.blue + 0.02;
  wipe.style.display = wiping ? 'block' : 'none';
  if (wiping) {
    const from = toScreen(view, wipeFrom.x, wipeFrom.y);
    const tall = ease.inOutCubic(span(time, T.wipe, T.wipe + 0.2));
    const wide = ease.inExpo(span(time, T.wipe + 0.1, T.blue - 0.01));
    const left = lerp(from.x, 0, wide);
    const top = lerp(from.y, 0, tall);
    wipe.style.left = `${left}px`;
    wipe.style.top = `${top}px`;
    wipe.style.width = `${lerp(from.x + wipeFrom.width, 1920, wide) - left}px`;
    wipe.style.height = `${lerp(from.y + wipeFrom.height, 1080, tall) - top}px`;
    wipe.style.boxShadow = `0 0 ${60 + 120 * tall}px rgba(37, 99, 235, 0.8)`;
  }

  const live = time >= T.blue;
  ending.style.display = live ? 'block' : 'none';
  world.style.display = live ? 'none' : 'block';
  if (!live) return;

  // The frame starts inside the solid part of the mark, so the cut from the
  // blue wipe is invisible and the logo is found by pulling back from it.
  const pull = settle(span(time, T.blue + 0.04, T.blue + 0.74), 5.2, 6.2);
  const dock = ease.glide(span(time, T.wordmark, T.wordmark + 0.8));
  const settled = clamp(pull);
  const scale = SOLID.scale * (ALONE.scale / SOLID.scale) ** pull * (LOCKUP.scale / ALONE.scale) ** dock;
  const anchor = { x: lerp(SOLID.x, MARK_CENTRE.x, settled), y: lerp(SOLID.y, MARK_CENTRE.y, settled) };
  const docked = { x: LOCKUP.left + MARK_CENTRE.x * LOCKUP.scale, y: LOCKUP.top + MARK_CENTRE.y * LOCKUP.scale };
  const at = {
    x: lerp(lerp(960, ALONE.x, settled), docked.x, dock),
    y: lerp(lerp(540, ALONE.y, settled), docked.y, dock),
  };
  markPath.setAttribute('transform', `translate(${at.x - anchor.x * scale} ${at.y - anchor.y * scale}) scale(${scale})`);

  const markRight = at.x + (315.6 - anchor.x) * scale;
  reveal.setAttribute('x', markRight + 4);
  letterPaths.forEach((path, index) => {
    const start = CUES.wordmark[index];
    const out = ease.outExpo(span(time, start, start + 0.6));
    path.setAttribute('transform', `translate(${LOCKUP.left - (1 - out) * 150} ${LOCKUP.top}) scale(${LOCKUP.scale})`);
    path.setAttribute('opacity', span(time, start, start + 0.12));
  });

  glow.style.opacity = 0.9 * pulse(time, T.blue + 0.1, 1.6) + 0.34 * span(time, T.blue + 0.2, T.wordmark + 0.6);
  glow.style.transform = `translateY(${-80 * dock}px) scale(${lerp(0.7, 1.25, dock)}, ${lerp(0.7, 0.72, dock)})`;

  const arrived = typedCount(CUES.tagline, time);
  const width = showTyped(tagline.text, arrived === 0 ? 0 : tagline.tokens[arrived - 1]);
  const typingDone = CUES.tagline.at(-1);
  tagline.caret.style.left = `${width + 6}px`;
  tagline.caret.style.opacity = time < T.tagline - 0.1 ? 0 : time < typingDone ? 1 : blink(time, typingDone);

  const shown = span(time, T.address, T.address + 0.5);
  address.style.opacity = ease.outCubic(shown);
  address.style.transform = `translateY(${(1 - ease.outExpo(shown)) * 14}px)`;

  ending.style.transform = `scale(${1 + 0.022 * ease.outCubic(span(time, T.blue + 0.7, DURATION))})`;
}

// ---------------------------------------------------------------- film

function measure() {
  const offsetIn = (node, root) => {
    let x = 0;
    let y = 0;
    for (let current = node; current && current !== root; current = current.offsetParent) {
      x += current.offsetLeft;
      y += current.offsetTop;
    }
    return { x, y };
  };

  for (const title of Object.values(titles)) for (const line of title.lines) measureTyped(line.text);
  for (const commit of COMMITS) measureTyped(commit.text);
  measureTyped(comment.text);
  measureTyped(tagline.text);

  const origin = rewrite.after.getBoundingClientRect().left;
  rewrite.tokens = [...rewrite.after.children].map((token) => token.getBoundingClientRect().right - origin);
  rewrite.width = rewrite.after.getBoundingClientRect().width;

  const glyph = pill.querySelector('.glyph');
  const pillAt = offsetIn(glyph, world);
  pillTarget = { x: pillAt.x + 11, y: pillAt.y + 11 };

  const last = titles.finish.lines.at(-1);
  const caretAtEnd = offsetIn(last.caret.parentNode, world);
  wipeFrom = { x: caretAtEnd.x + last.text.width + 12, y: caretAtEnd.y + 18, width: 8, height: 104 };
}

function seek(time) {
  const now = clamp(time, 0, DURATION);
  const view = camera(now);
  world.style.transform = `scale(${view.zoom}) translate(${-view.x}px, ${-view.y}px)`;

  drawOpening(now);
  updatePane(now, view);

  const reviewing = now >= T.land && now < T.commit + 0.02;
  titles.review.node.style.display = reviewing ? 'block' : 'none';
  if (reviewing) updateTitle(titles.review, now, CUES.reviewTitle);

  updateGraph(now);
  updatePullRequest(now);
  updateDot(now, view);
  updateEnding(now, view);

  timecode.style.display = params.has('timecode') ? 'block' : 'none';
  timecode.textContent = now.toFixed(3);
  probe.style.display = params.has('probe') ? 'block' : 'none';
  probe.style.left = `${FILM.probeColumn(now) - 2}px`;
}

// How fast the picture moves, so the renderer can spend exposures where the
// blur needs them: 2 for the zoom, the whip pans and the wipe, 0 for the hold.
function motion(time) {
  const fast = [
    [T.agent, T.period + 0.35],
    [T.zoom + 0.6, T.land + 0.6],
    [T.collapse, T.commit + 0.3],
    [T.panUp[0] - 0.05, T.pullRequest + 0.3],
    [T.wipe, T.blue + 0.6],
  ];
  if (fast.some(([from, to]) => time >= from && time <= to)) return 2;
  return time > T.address + 0.6 ? 0 : 1;
}

const FILM = {
  duration: DURATION,
  seek,
  motion,
  probeColumn: (time) => 18 + (Math.round(time * 240) % 400) * 4,
  ready: null,
};
window.FILM = FILM;

FILM.ready = (async () => {
  const fonts = ['600 128px Poppins', '500 43px Poppins', '400 26px Lilex', '400 20px Inter', '500 30px Inter', '600 21px Inter'];
  await Promise.all(fonts.map((font) => document.fonts.load(font)));
  await document.fonts.ready;

  buildWall();
  buildComment();
  buildOpening();
  measure();
  seek(0);

  if (!rendering) play();
})();

// Opening film.html in a browser plays it in real time: space pauses, the
// arrow keys step a frame, and the soundtrack follows if it has been rendered.
function play() {
  const fit = () => {
    const scale = Math.min(innerWidth / 1920, innerHeight / 1080);
    stage.style.transform = `translate(${(innerWidth - 1920 * scale) / 2}px, ${(innerHeight - 1080 * scale) / 2}px) scale(${scale})`;
  };
  addEventListener('resize', fit);
  fit();

  const bar = el('div', '', document.body, '<div class="played"></div>');
  bar.id = 'player';
  const audio = new Audio('dist/soundtrack.wav');
  let playing = true;
  let position = 0;
  let previous = performance.now();

  const start = () => {
    audio.currentTime = position;
    // The soundtrack is optional, and browsers refuse to play it before a click.
    audio.play().catch(() => {});
  };
  addEventListener('keydown', (event) => {
    if (event.key === ' ') {
      playing = !playing;
      if (playing) start();
      else audio.pause();
    } else if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
      playing = false;
      audio.pause();
      position = clamp(position + (event.key === 'ArrowRight' ? 1 : -1) / 60, 0, DURATION);
    }
  });
  addEventListener('pointerdown', start, { once: true });

  const frame = (now) => {
    if (playing) {
      position += (now - previous) / 1000;
      if (position >= DURATION) {
        position = 0;
        start();
      }
    }
    previous = now;
    seek(position);
    bar.firstChild.style.width = `${(position / DURATION) * 100}%`;
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
