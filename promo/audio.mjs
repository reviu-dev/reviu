#!/usr/bin/env node
// Synthesizes the soundtrack from the cue sheet the picture uses, so every
// sound lands on the frame it belongs to. Nothing is sampled: it is all sines,
// filtered noise and a small reverb.

import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BEAT, CUES, DURATION, T, beat, random } from './cues.mjs';

const RATE = 48000;
const LENGTH = Math.round(DURATION * RATE);
const TAU = Math.PI * 2;

const stereo = () => [new Float32Array(LENGTH), new Float32Array(LENGTH)];
const drums = stereo();
const bass = stereo();
const pad = stereo();
const effects = stereo();
const space = stereo();

// Screen positions double as stereo positions: titles sit left, the diff,
// graph and pull request sit right.
const LEFT = -0.45;
const RIGHT = 0.32;

const NOTE = {
  A1: 55, F1: 43.65, G1: 49,
  A2: 110, F2: 87.31, G2: 98, E3: 164.81,
  A3: 220, B3: 246.94, C4: 261.63, Cs4: 277.18, D4: 293.66, E4: 329.63, F4: 349.23, G4: 392,
  A4: 440, B4: 493.88, D5: 587.33, E5: 659.25, G5: 783.99, A5: 880, B5: 987.77,
  Cs6: 1108.73, D6: 1174.66, E6: 1318.51, A6: 1760,
};

function play(target, start, length, voice, { gain = 1, pan = 0, send = 0 } = {}) {
  const first = Math.round(start * RATE);
  const count = Math.min(Math.round(length * RATE), LENGTH - first);
  const left = gain * Math.cos(((pan + 1) * Math.PI) / 4);
  const right = gain * Math.sin(((pan + 1) * Math.PI) / 4);
  for (let index = 0; index < count; index++) {
    const time = index / RATE;
    // A short fade at the end of every voice keeps it from clicking off.
    const sample = voice(time) * Math.min(1, (length - time) / 0.008);
    target[0][first + index] += sample * left;
    target[1][first + index] += sample * right;
    if (send > 0) {
      space[0][first + index] += sample * left * send;
      space[1][first + index] += sample * right * send;
    }
  }
}

// ---------------------------------------------------------------- voices

function kick() {
  let phase = 0;
  return (time) => {
    phase += (TAU * (47 + 105 * Math.exp(-time * 42))) / RATE;
    return Math.sin(phase) * Math.min(1, time / 0.0015) * Math.exp(-time * 9);
  };
}

function boom(from, to, fall, decay) {
  let phase = 0;
  return (time) => {
    phase += (TAU * (to + (from - to) * Math.exp(-time * fall))) / RATE;
    return Math.sin(phase) * Math.min(1, time / 0.004) * Math.exp(-time * decay);
  };
}

function chirp(from, to, length, curve = 1) {
  let phase = 0;
  return (time) => {
    const amount = Math.min(1, time / length) ** curve;
    phase += (TAU * from * (to / from) ** amount) / RATE;
    return Math.sin(phase) * Math.min(1, time / 0.003) * Math.sin(Math.PI * Math.min(1, time / length)) ** 0.6;
  };
}

function bell(frequency, decay = 7) {
  const partials = [[1, 1], [2, 0.3], [3.01, 0.13], [4.17, 0.06]];
  return (time) => {
    let sample = 0;
    partials.forEach(([ratio, level], index) => {
      sample += level * Math.sin(TAU * frequency * ratio * time) * Math.exp(-time * decay * (1 + index * 0.8));
    });
    return sample * Math.min(1, time / 0.002);
  };
}

// The agent's tokens: a bright, clean tick.
function tick(frequency, seed) {
  const noise = random(seed);
  return (time) =>
    (Math.sin(TAU * frequency * time) * Math.exp(-time * 170) + (noise() * 2 - 1) * 0.3 * Math.exp(-time * 520)) *
    Math.min(1, time / 0.0004);
}

// A person's keystrokes: lower, softer and noisier than a tick.
function tock(frequency, seed) {
  const noise = random(seed);
  let low = 0;
  return (time) => {
    low += 0.22 * (noise() * 2 - 1 - low);
    return (Math.sin(TAU * frequency * time) * 0.55 * Math.exp(-time * 210) + low * 2.2 * Math.exp(-time * 150)) * Math.min(1, time / 0.0006);
  };
}

function noiseBand(seed, cutoffAt, resonance = 0.5) {
  const noise = random(seed);
  let low = 0;
  let band = 0;
  return (time) => {
    const tuning = 2 * Math.sin((Math.PI * Math.min(9000, cutoffAt(time))) / RATE);
    low += tuning * band;
    band += tuning * (noise() * 2 - 1 - low - resonance * band);
    return band;
  };
}

function whoosh(seed, from, to, length, shape) {
  const band = noiseBand(seed, (time) => from * (to / from) ** Math.min(1, time / length), 0.45);
  return (time) => band(time) * shape(Math.min(1, time / length));
}

function hat(seed) {
  const noise = random(seed);
  let low = 0;
  return (time) => {
    const input = noise() * 2 - 1;
    low += 0.55 * (input - low);
    return (input - low) * Math.exp(-time * 120);
  };
}

function snap(seed) {
  const band = noiseBand(seed, () => 1900, 0.3);
  return (time) => band(time) * Math.exp(-time * 60) * Math.min(1, time / 0.001);
}

function pluck(frequency) {
  return (time) => {
    const tone = Math.sin(TAU * frequency * time) + 0.4 * Math.sin(TAU * frequency * 2 * time) + 0.15 * Math.sin(TAU * frequency * 3 * time);
    return Math.tanh(1.5 * tone * Math.min(1, time / 0.006) * Math.exp(-time * 7.5)) / Math.tanh(1.5);
  };
}

// A sustained chord. Each note is split into two slightly detuned sines, one
// per ear, which is what makes it sound wide rather than like an organ stop.
function sustain(target, start, length, notes, { gain, attack, release, send = 0 }) {
  const first = Math.round(start * RATE);
  const count = Math.min(Math.round(length * RATE), LENGTH - first);
  for (let index = 0; index < count; index++) {
    const time = index / RATE;
    const envelope = Math.min(1, time / attack) ** 1.5 * Math.min(1, (length - time) / release) ** 1.5;
    for (let channel = 0; channel < 2; channel++) {
      const detune = channel === 0 ? 0.9975 : 1.0025;
      let sample = 0;
      notes.forEach(([frequency, level = 1]) => {
        const phase = TAU * frequency * time;
        sample += level * (0.55 * Math.sin(phase * detune) + 0.45 * Math.sin(phase) + 0.16 * Math.sin(phase * 2 * detune));
      });
      const value = (sample / notes.length) * envelope * gain;
      target[channel][first + index] += value;
      space[channel][first + index] += value * send;
    }
  }
}

// ---------------------------------------------------------------- score

const next = random(11);
const between = (low, high) => low + (high - low) * next();
let voiceSeed = 100;
const seed = () => voiceSeed++;

// Opening: a caret on a black stage, then the agent's first three tokens.
play(effects, 0, 0.7, boom(92, 52, 18, 6), { gain: 0.34 });
play(effects, 0, 1.4, bell(NOTE.A5, 4), { gain: 0.045, send: 0.9 });
sustain(pad, 0, T.land + 0.05, [[NOTE.A2], [NOTE.E3, 0.7]], { gain: 0.09, attack: 2.6, release: 0.05, send: 0.3 });
CUES.headline.flat().forEach((time, index) => {
  play(effects, time, 0.06, tick(2050 + index * 70, seed()), { gain: 0.1, pan: LEFT + index * 0.04, send: 0.15 });
});
play(effects, T.agent, 0.3, boom(125, 66, 28, 14), { gain: 0.3 });
play(effects, T.writes, 0.3, boom(125, 66, 28, 14), { gain: 0.3 });
play(effects, T.period, 0.5, boom(175, 78, 26, 9), { gain: 0.46 });
play(effects, T.period, 0.06, tick(1250, seed()), { gain: 0.09, send: 0.3 });
play(effects, T.glass, 1.2, bell(NOTE.E6, 5), { gain: 0.022, send: 1 });

// The dive into the code: a riser that is cut dead on the landing.
const dive = T.land - T.zoom;
play(effects, T.zoom, dive, whoosh(seed(), 240, 7200, dive, (amount) => amount ** 2.3), { gain: 0.3, send: 0.25 });
play(effects, T.zoom, dive, chirp(NOTE.A3, NOTE.A5, dive * 1.02, 1.6), { gain: 0.07, send: 0.4 });
for (let time = T.zoom, gap = 0.13; time < T.land - 0.03; time += gap, gap = Math.max(0.026, gap * 0.93)) {
  const amount = (time - T.zoom) / dive;
  play(effects, time, 0.05, tick(between(1700, 3400), seed()), { gain: 0.018 + 0.06 * amount, pan: between(-0.6, 0.6), send: 0.2 });
}
CUES.hunk.forEach((time, index) => {
  play(effects, time, 0.05, tick(2400 + index * 45, seed()), { gain: 0.07, pan: 0.1, send: 0.15 });
});

// The groove: sixteen beats under the review, the graph and the pull request.
const kicks = [];
for (let count = 8; count < 24; count++) {
  const root = count < 16 ? ['A2', 'A1'] : count < 20 ? ['F2', 'F1'] : ['G2', 'G1'];
  kicks.push(beat(count));
  play(drums, beat(count), 0.4, kick(), { gain: 0.86 });
  play(bass, beat(count + 0.5), 0.24, pluck(NOTE[root[0]]), { gain: 0.36 });
  play(bass, beat(count + 0.5), 0.24, pluck(NOTE[root[1]]), { gain: 0.22 });
  play(drums, beat(count + 0.5), 0.06, hat(seed()), { gain: 0.085, pan: 0.15 });
  play(drums, beat(count + 0.25), 0.04, hat(seed()), { gain: 0.03, pan: -0.2 });
  play(drums, beat(count + 0.75), 0.04, hat(seed()), { gain: 0.036, pan: 0.25 });
  if (count % 2 === 1) play(drums, beat(count), 0.12, snap(seed()), { gain: 0.11, send: 0.35 });
}
sustain(pad, T.land, beat(8) + 0.1, [[NOTE.A3], [NOTE.C4], [NOTE.E4]], { gain: 0.15, attack: 0.25, release: 0.2, send: 0.35 });
sustain(pad, T.commit, beat(4) + 0.1, [[NOTE.A3], [NOTE.C4], [NOTE.F4]], { gain: 0.15, attack: 0.12, release: 0.2, send: 0.35 });
sustain(pad, T.pullRequest, beat(4) + 0.05, [[NOTE.B3], [NOTE.D4], [NOTE.G4]], { gain: 0.15, attack: 0.12, release: 0.15, send: 0.35 });

// Landing in the diff.
play(effects, T.land, 1.4, boom(78, 36, 6, 3.4), { gain: 0.5 });
kicks.unshift(T.land);

// You review: keys on the left for the title, the comment on the right.
CUES.reviewTitle.forEach((time) => play(effects, time, 0.07, tock(between(950, 1250), seed()), { gain: 0.085, pan: LEFT }));
T.steps.forEach((time, index) => play(effects, time, 0.06, tick(1500 + index * 210, seed()), { gain: 0.085, pan: RIGHT, send: 0.2 }));
play(effects, T.thread, 0.09, chirp(430, 700, 0.08), { gain: 0.1, pan: RIGHT, send: 0.3 });
CUES.comment.forEach((time) => play(effects, time, 0.07, tock(between(1000, 1350), seed()), { gain: 0.07, pan: RIGHT }));
play(effects, T.send, 0.6, bell(NOTE.E5, 9), { gain: 0.085, pan: RIGHT, send: 0.6 });
play(effects, T.send + 0.075, 0.7, bell(NOTE.A5, 8), { gain: 0.085, pan: RIGHT, send: 0.6 });
play(effects, T.strike, 0.15, chirp(900, 340, 0.14), { gain: 0.06, pan: RIGHT, send: 0.2 });
CUES.rewrite.forEach((time, index) => {
  play(effects, time, 0.05, tick(2300 + (index % 5) * 90, seed()), { gain: 0.075, pan: RIGHT, send: 0.15 });
});
play(effects, T.fold, 0.1, chirp(620, 380, 0.09), { gain: 0.07, pan: RIGHT, send: 0.2 });
play(effects, T.staged, 0.8, bell(NOTE.A5, 8), { gain: 0.08, pan: RIGHT, send: 0.6 });

// The diff becomes a commit and the camera whips right to the graph.
play(effects, T.collapse, 0.32, chirp(1300, 190, 0.3, 0.7), { gain: 0.075, pan: RIGHT, send: 0.3 });
play(effects, T.panRight[0] - 0.06, 0.46, whoosh(seed(), 500, 5200, 0.3, (amount) => Math.sin(Math.PI * amount) ** 1.4), { gain: 0.3, send: 0.25 });
play(effects, T.commit, 0.9, bell(NOTE.A4, 6), { gain: 0.12, pan: RIGHT, send: 0.6 });
CUES.shipTitle.forEach((time) => play(effects, time, 0.05, tick(between(1900, 2300), seed()), { gain: 0.04, pan: LEFT }));
CUES.commitMessage.forEach((time, index) => {
  if (index % 3 === 0) play(effects, time, 0.05, tick(between(2500, 3000), seed()), { gain: 0.035, pan: RIGHT });
});

// Rebase: a swish as the commits change places, and a flutter as their hashes rewrite.
play(effects, T.rebase, 0.3, whoosh(seed(), 1400, 5200, 0.26, (amount) => Math.sin(Math.PI * amount) ** 2), { gain: 0.17, pan: RIGHT, send: 0.3 });
for (let time = T.rebase + 0.04; time < T.rebase + 0.42; time += 1 / 30) {
  play(effects, time, 0.03, tick(between(2600, 4600), seed()), { gain: 0.026, pan: RIGHT });
}

// Push: a launch, then the camera chases it up to GitHub.
play(effects, T.push + 0.05, 0.2, chirp(460, 2100, 0.18, 1.3), { gain: 0.09, pan: 0.15, send: 0.4 });
play(effects, T.panUp[0] - 0.08, 0.6, whoosh(seed(), 380, 6000, 0.5, (amount) => Math.sin(Math.PI * amount ** 1.3) ** 1.4), { gain: 0.3, send: 0.25 });
play(effects, T.pullRequest, 0.9, bell(NOTE.D5, 6), { gain: 0.075, pan: RIGHT, send: 0.6 });
CUES.finishTitle.forEach((time) => play(effects, time, 0.05, tick(between(1900, 2300), seed()), { gain: 0.04, pan: LEFT }));

// Checks pass on a rising arpeggio, then the merge.
[NOTE.D5, NOTE.G5, NOTE.B5].forEach((note, index) => {
  play(effects, T.checks[index], 1.1, bell(note, 6), { gain: 0.115, pan: RIGHT, send: 0.7 });
});
play(effects, T.press, 0.07, tock(1150, seed()), { gain: 0.11, pan: RIGHT });
[NOTE.G4, NOTE.B4, NOTE.D5, NOTE.G5, NOTE.D6].forEach((note, index) => {
  play(effects, T.merged + index * 0.014, 1.3, bell(note, 4.5), { gain: index === 4 ? 0.04 : 0.085, pan: RIGHT - 0.1 + index * 0.05, send: 0.8 });
});

// The caret wipes the frame blue and the logo is found by pulling back from it.
const sweep = T.blue - T.wipe;
play(effects, T.wipe, sweep, whoosh(seed(), 600, 7600, sweep, (amount) => amount ** 1.6), { gain: 0.34, send: 0.3 });
play(effects, T.wipe, sweep, chirp(NOTE.A3, NOTE.A5, sweep * 1.02, 1.4), { gain: 0.07, send: 0.4 });
play(drums, T.blue, 0.4, kick(), { gain: 0.9 });
kicks.push(T.blue);
play(effects, T.blue, 2.6, boom(96, 31, 4.5, 1.7), { gain: 0.72 });
play(effects, T.blue, 1.8, whoosh(seed(), 5200, 700, 0.9, (amount) => (1 - amount) ** 2.2), { gain: 0.26, send: 0.6 });
sustain(pad, T.blue, DURATION - T.blue, [[NOTE.A2, 0.9], [NOTE.E3, 0.8], [NOTE.A3], [NOTE.Cs4], [NOTE.E4], [NOTE.A4, 0.7], [NOTE.B4, 0.35]], {
  gain: 0.3,
  attack: 0.7,
  release: 1.5,
  send: 0.5,
});

// Wordmark, tagline and address.
[NOTE.E5, NOTE.A5, NOTE.Cs6, NOTE.E6, NOTE.A6].forEach((note, index) => {
  play(effects, CUES.wordmark[index], 1.6, bell(note, 3.6), { gain: 0.05, pan: -0.35 + index * 0.18, send: 1 });
});
CUES.tagline.forEach((time, index) => {
  play(effects, time, 0.05, tick(2100 + (index % 4) * 110, seed()), { gain: 0.055, pan: -0.3 + index * 0.085, send: 0.2 });
});
play(effects, T.address, 1.6, bell(NOTE.E5, 3.5), { gain: 0.05, send: 1 });

// ---------------------------------------------------------------- mix

// Freeverb: eight damped combs into four all-pass filters per channel.
function reverb(input, { room = 0.84, damping = 0.3 } = {}) {
  const output = stereo();
  const scale = RATE / 44100;
  for (let channel = 0; channel < 2; channel++) {
    const spread = channel * 23;
    const combs = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617].map((size) => ({
      buffer: new Float32Array(Math.round(size * scale) + spread),
      index: 0,
      store: 0,
    }));
    const passes = [556, 441, 341, 225].map((size) => ({
      buffer: new Float32Array(Math.round(size * scale) + spread),
      index: 0,
    }));
    for (let index = 0; index < LENGTH; index++) {
      const dry = (input[0][index] + input[1][index]) * 0.015;
      let sample = 0;
      for (const comb of combs) {
        const delayed = comb.buffer[comb.index];
        comb.store = delayed * (1 - damping) + comb.store * damping;
        comb.buffer[comb.index] = dry + comb.store * room;
        comb.index = (comb.index + 1) % comb.buffer.length;
        sample += delayed;
      }
      for (const pass of passes) {
        const delayed = pass.buffer[pass.index];
        pass.buffer[pass.index] = sample + delayed * 0.5;
        pass.index = (pass.index + 1) % pass.buffer.length;
        sample = delayed - sample;
      }
      output[channel][index] = sample;
    }
  }
  return output;
}

// Every kick pushes the bass and the chords down for a moment, which is what
// makes a groove feel like it breathes.
const duck = new Float32Array(LENGTH).fill(1);
for (const time of kicks) {
  const first = Math.round(time * RATE);
  for (let index = first; index < Math.min(LENGTH, first + RATE * 0.5); index++) {
    duck[index] = Math.min(duck[index], 1 - 0.62 * Math.exp(-(index - first) / RATE / 0.085));
  }
}

const wet = reverb(space);
const mixed = stereo();
let peak = 0;
for (let channel = 0; channel < 2; channel++) {
  for (let index = 0; index < LENGTH; index++) {
    const time = index / RATE;
    const fade = Math.min(1, time / 0.004) * Math.min(1, ((DURATION - 0.02 - time) / 0.9) ** 1.4 || 0);
    const sample =
      drums[channel][index] +
      (bass[channel][index] + pad[channel][index]) * duck[index] +
      effects[channel][index] +
      wet[channel][index] * 2.4;
    mixed[channel][index] = Math.tanh(sample * 1.15) * Math.max(0, fade);
    peak = Math.max(peak, Math.abs(mixed[channel][index]));
  }
}

const level = (signal) => {
  let sum = 0;
  for (const channel of signal) for (const sample of channel) sum += sample * sample;
  return `${(10 * Math.log10(sum / (2 * LENGTH) + 1e-12)).toFixed(1)} dB`;
};
console.log(
  `Levels: drums ${level(drums)}, bass ${level(bass)}, chords ${level(pad)}, effects ${level(effects)}, reverb ${level(wet)}`,
);

const gain = 10 ** (-2 / 20) / peak;
const data = Buffer.alloc(LENGTH * 6);
for (let index = 0; index < LENGTH; index++) {
  for (let channel = 0; channel < 2; channel++) {
    data.writeIntLE(Math.round(mixed[channel][index] * gain * 8388607), (index * 2 + channel) * 3, 3);
  }
}

const header = Buffer.alloc(44);
header.write('RIFF', 0, 'latin1');
header.writeUInt32LE(36 + data.length, 4);
header.write('WAVEfmt ', 8, 'latin1');
header.writeUInt32LE(16, 16);
header.writeUInt16LE(1, 20);
header.writeUInt16LE(2, 22);
header.writeUInt32LE(RATE, 24);
header.writeUInt32LE(RATE * 6, 28);
header.writeUInt16LE(6, 32);
header.writeUInt16LE(24, 34);
header.write('data', 36, 'latin1');
header.writeUInt32LE(data.length, 40);

const dist = path.join(path.dirname(fileURLToPath(import.meta.url)), 'dist');
mkdirSync(dist, { recursive: true });
const file = path.join(dist, 'soundtrack.wav');
writeFileSync(file, Buffer.concat([header, data]));
console.log(`Wrote ${path.relative(process.cwd(), file)} (${(BEAT * 32).toFixed(1)}s, ${Math.round(60 / BEAT)} bpm)`);
