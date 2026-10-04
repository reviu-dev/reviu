#!/usr/bin/env node
// Renders film.html to video.
//
// Chromium is driven over the DevTools protocol and asked for the picture at an
// exact time, so a render is identical on every run and does not depend on how
// fast the machine can draw. Motion blur comes from averaging several sub-frame
// exposures in linear light, which is what a camera shutter does.

import { fork, spawn } from 'node:child_process';
import { once } from 'node:events';
import {
  createReadStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { createServer } from 'node:http';
import { availableParallelism, homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..');
const distDir = path.join(here, 'dist');

const WIDTH = 1920;
const HEIGHT = 1080;

// The film borrows the fonts the product already ships, so the server exposes
// those directories and nothing else from the repository.
const SERVED = [
  'promo/',
  'desktop/crates/ui/assets/fonts/',
  'website/node_modules/@fontsource/poppins/files/',
];

const REQUIRED_FONTS = [
  'desktop/crates/ui/assets/fonts/inter/Inter-Medium.otf',
  'desktop/crates/ui/assets/fonts/lilex/Lilex-Regular.ttf',
  'website/node_modules/@fontsource/poppins/files/poppins-latin-600-normal.woff2',
];

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.otf': 'font/otf',
  '.ttf': 'font/ttf',
  '.woff2': 'font/woff2',
  '.wav': 'audio/wav',
};

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index];
    if (!arg.startsWith('--')) throw new Error(`Unexpected argument: ${arg}`);
    const next = argv[index + 1];
    if (next === undefined || next.startsWith('--')) {
      args[arg.slice(2)] = true;
    } else {
      args[arg.slice(2)] = next;
      index++;
    }
  }
  return args;
}

function startServer() {
  const server = createServer((request, response) => {
    const relative = decodeURIComponent(new URL(request.url, 'http://film').pathname).replace(/^\/+/, '');
    const file = path.join(repoRoot, path.normalize(relative));
    const allowed = SERVED.some((prefix) => relative.startsWith(prefix)) && file.startsWith(repoRoot);
    if (!allowed || !existsSync(file) || !statSync(file).isFile()) {
      response.writeHead(404).end('not found');
      return;
    }
    response.writeHead(200, {
      'content-type': MIME[path.extname(file)] ?? 'application/octet-stream',
      'cache-control': 'no-store',
    });
    createReadStream(file).pipe(response);
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

function findChromium() {
  if (process.env.CHROME_BIN) return { bin: process.env.CHROME_BIN, headlessFlag: '--headless=new' };

  const cache = path.join(homedir(), 'Library/Caches/ms-playwright');
  if (existsSync(cache)) {
    const shells = readdirSync(cache)
      .filter((name) => name.startsWith('chromium_headless_shell-'))
      .sort((a, b) => Number(b.split('-')[1]) - Number(a.split('-')[1]));
    for (const shell of shells) {
      for (const platform of readdirSync(path.join(cache, shell))) {
        const bin = path.join(cache, shell, platform, 'chrome-headless-shell');
        if (existsSync(bin)) return { bin, headlessFlag: '--headless' };
      }
    }
  }

  const installed = [
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
  ].find(existsSync);
  if (installed) return { bin: installed, headlessFlag: '--headless=new' };

  throw new Error('No Chromium found. Set CHROME_BIN to a Chrome or Chromium binary.');
}

class DevTools {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 0;
    this.pending = new Map();
    this.handlers = new Map();
    socket.addEventListener('message', (event) => this.receive(JSON.parse(event.data)));
  }

  static async connect(endpoint) {
    const socket = new WebSocket(endpoint);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', () => reject(new Error('DevTools socket failed')), { once: true });
    });
    return new DevTools(socket);
  }

  receive(message) {
    if (message.id !== undefined) {
      const request = this.pending.get(message.id);
      this.pending.delete(message.id);
      if (message.error) request.reject(new Error(`${request.method}: ${message.error.message}`));
      else request.resolve(message.result);
      return;
    }
    for (const handler of this.handlers.get(message.method) ?? []) handler(message.params);
  }

  send(method, params = {}, sessionId) {
    const id = ++this.nextId;
    this.socket.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject, method }));
  }

  on(method, handler) {
    if (!this.handlers.has(method)) this.handlers.set(method, []);
    this.handlers.get(method).push(handler);
  }

  next(method) {
    return new Promise((resolve) => {
      const handlers = this.handlers.get(method) ?? [];
      const handler = (params) => {
        handlers.splice(handlers.indexOf(handler), 1);
        resolve(params);
      };
      handlers.push(handler);
      this.handlers.set(method, handlers);
    });
  }
}

async function openFilm({ url, scale }) {
  const { bin, headlessFlag } = findChromium();
  const profile = mkdtempSync(path.join(tmpdir(), 'reviu-film-'));
  const chromium = spawn(
    bin,
    [
      headlessFlag,
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      '--no-first-run',
      '--no-default-browser-check',
      '--hide-scrollbars',
      '--mute-audio',
      // Without these the pixels depend on the display profile and on
      // subpixel text rendering, and two machines would not render alike.
      '--force-color-profile=srgb',
      '--disable-lcd-text',
      '--font-render-hinting=none',
      `--window-size=${WIDTH},${HEIGHT}`,
      'about:blank',
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] },
  );

  const endpoint = await new Promise((resolve, reject) => {
    let output = '';
    chromium.stderr.on('data', (chunk) => {
      output += chunk;
      const match = output.match(/DevTools listening on (ws:\/\/\S+)/);
      if (match) resolve(match[1]);
    });
    chromium.once('exit', (code) => reject(new Error(`Chromium exited early (code ${code})\n${output}`)));
  });

  const devtools = await DevTools.connect(endpoint);
  const { targetId } = await devtools.send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await devtools.send('Target.attachToTarget', { targetId, flatten: true });
  const send = (method, params) => devtools.send(method, params, sessionId);

  devtools.on('Runtime.exceptionThrown', ({ exceptionDetails }) => {
    console.error('[film]', exceptionDetails.exception?.description ?? exceptionDetails.text);
  });
  devtools.on('Runtime.consoleAPICalled', ({ type, args }) => {
    if (type === 'error' || type === 'warning') {
      console.error(`[film ${type}]`, args.map((arg) => arg.value ?? arg.description).join(' '));
    }
  });

  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', {
    width: WIDTH,
    height: HEIGHT,
    deviceScaleFactor: scale,
    mobile: false,
  });

  const loaded = devtools.next('Page.loadEventFired');
  await send('Page.navigate', { url });
  await loaded;

  const evaluate = async (expression, awaitPromise = false) => {
    const { result, exceptionDetails } = await send('Runtime.evaluate', {
      expression,
      awaitPromise,
      returnByValue: true,
    });
    if (exceptionDetails) {
      throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
    }
    return result.value;
  };

  await evaluate('FILM.ready', true);

  return {
    evaluate,
    async capture(time) {
      await evaluate(`FILM.seek(${time})`);
      const { data } = await send('Page.captureScreenshot', { format: 'png', optimizeForSpeed: true });
      return Buffer.from(data, 'base64');
    },
    async close() {
      chromium.kill();
      await once(chromium, 'exit');
      rmSync(profile, { recursive: true, force: true });
    },
  };
}

function decodePng(buffer) {
  let width = 0;
  let height = 0;
  let channels = 0;
  const data = [];
  for (let position = 8; position < buffer.length; ) {
    const length = buffer.readUInt32BE(position);
    const type = buffer.toString('latin1', position + 4, position + 8);
    const body = position + 8;
    if (type === 'IHDR') {
      width = buffer.readUInt32BE(body);
      height = buffer.readUInt32BE(body + 4);
      const depth = buffer[body + 8];
      const colorType = buffer[body + 9];
      channels = colorType === 6 ? 4 : colorType === 2 ? 3 : 0;
      if (depth !== 8 || channels === 0 || buffer[body + 12] !== 0) {
        throw new Error(`Unsupported PNG (depth ${depth}, colour type ${colorType})`);
      }
    } else if (type === 'IDAT') {
      data.push(buffer.subarray(body, body + length));
    } else if (type === 'IEND') {
      break;
    }
    position = body + length + 4;
  }

  const filtered = zlib.inflateSync(data.length === 1 ? data[0] : Buffer.concat(data));
  const stride = width * channels;
  const pixels = Buffer.allocUnsafe(stride * height);

  for (let row = 0; row < height; row++) {
    const filter = filtered[row * (stride + 1)];
    const source = row * (stride + 1) + 1;
    const target = row * stride;
    const above = target - stride;
    switch (filter) {
      case 0:
        filtered.copy(pixels, target, source, source + stride);
        break;
      case 1:
        for (let x = 0; x < stride; x++) {
          const left = x >= channels ? pixels[target + x - channels] : 0;
          pixels[target + x] = filtered[source + x] + left;
        }
        break;
      case 2:
        for (let x = 0; x < stride; x++) {
          const up = row > 0 ? pixels[above + x] : 0;
          pixels[target + x] = filtered[source + x] + up;
        }
        break;
      case 3:
        for (let x = 0; x < stride; x++) {
          const left = x >= channels ? pixels[target + x - channels] : 0;
          const up = row > 0 ? pixels[above + x] : 0;
          pixels[target + x] = filtered[source + x] + ((left + up) >> 1);
        }
        break;
      case 4:
        for (let x = 0; x < stride; x++) {
          const left = x >= channels ? pixels[target + x - channels] : 0;
          const up = row > 0 ? pixels[above + x] : 0;
          const corner = row > 0 && x >= channels ? pixels[above + x - channels] : 0;
          const estimate = left + up - corner;
          const toLeft = Math.abs(estimate - left);
          const toUp = Math.abs(estimate - up);
          const toCorner = Math.abs(estimate - corner);
          const nearest = toLeft <= toUp && toLeft <= toCorner ? left : toUp <= toCorner ? up : corner;
          pixels[target + x] = filtered[source + x] + nearest;
        }
        break;
      default:
        throw new Error(`Unknown PNG filter ${filter}`);
    }
  }
  return { width, height, channels, pixels };
}

function encodePng(rgb, width, height) {
  const stride = width * 3;
  const filtered = Buffer.alloc((stride + 1) * height);
  for (let row = 0; row < height; row++) {
    rgb.copy(filtered, row * (stride + 1) + 1, row * stride, (row + 1) * stride);
  }
  const chunk = (type, body) => {
    const tagged = Buffer.concat([Buffer.from(type, 'latin1'), body]);
    const frame = Buffer.alloc(tagged.length + 8);
    frame.writeUInt32BE(body.length, 0);
    tagged.copy(frame, 4);
    frame.writeUInt32BE(zlib.crc32(tagged), tagged.length + 4);
    return frame;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', zlib.deflateSync(filtered, { level: 4 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const LINEAR_STEPS = 4096;
const SRGB_TO_LINEAR = Float32Array.from({ length: 256 }, (_, value) => {
  const channel = value / 255;
  return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
});
const LINEAR_TO_SRGB = Float32Array.from({ length: LINEAR_STEPS + 2 }, (_, step) => {
  const linear = Math.min(1, step / LINEAR_STEPS);
  return 255 * (linear <= 0.0031308 ? linear * 12.92 : 1.055 * linear ** (1 / 2.4) - 0.055);
});

// Averages exposures in linear light. Averaging the encoded sRGB values instead
// would darken every blurred edge, which is why cheap motion blur looks muddy.
class Exposure {
  constructor(width, height) {
    this.width = width;
    this.height = height;
    this.light = new Float32Array(width * height * 3);
    this.count = 0;
    this.single = null;
  }

  add({ pixels, channels }) {
    if (this.count === 0) this.single = { pixels, channels };
    const { light } = this;
    for (let source = 0, target = 0; target < light.length; source += channels, target += 3) {
      light[target] += SRGB_TO_LINEAR[pixels[source]];
      light[target + 1] += SRGB_TO_LINEAR[pixels[source + 1]];
      light[target + 2] += SRGB_TO_LINEAR[pixels[source + 2]];
    }
    this.count++;
  }

  develop(frame, grain) {
    const { light, count } = this;
    const rgb = Buffer.allocUnsafe(light.length);
    const single = count === 1 ? this.single : null;
    const scale = LINEAR_STEPS / count;
    let state = (frame + 1) * 0x9e3779b1;
    const noise = () => {
      state ^= state << 13;
      state ^= state >>> 17;
      state ^= state << 5;
      return (state >>> 0) / 4294967296 - 0.5;
    };

    for (let pixel = 0, source = 0; pixel < light.length; pixel += 3) {
      // One grain value per pixel keeps the texture neutral instead of
      // sparkling in colour.
      const speck = (noise() + noise()) * grain;
      for (let channel = 0; channel < 3; channel++) {
        let value;
        if (single) {
          value = single.pixels[source + channel];
        } else {
          const position = light[pixel + channel] * scale;
          const step = position | 0;
          value = LINEAR_TO_SRGB[step] + (LINEAR_TO_SRGB[step + 1] - LINEAR_TO_SRGB[step]) * (position - step);
          // Half a level of noise before rounding keeps blurred gradients
          // from banding; exact blacks and whites round back to themselves.
          value += noise();
        }
        // Grain fades out towards black so the stage stays truly black.
        value += speck * Math.min(1, value / 8);
        rgb[pixel + channel] = Math.max(0, Math.min(255, Math.round(value)));
      }
      if (single) source += single.channels;
    }

    light.fill(0);
    this.count = 0;
    this.single = null;
    return rgb;
  }
}

function sampleTimes(frame, samples, { fps, shutter, duration }) {
  const centre = frame / fps;
  if (samples === 1) return [centre];
  return Array.from({ length: samples }, (_, index) => {
    const offset = ((index + 0.5) / samples - 0.5) * (shutter / fps);
    return Math.max(0, Math.min(duration, centre + offset));
  });
}

async function exposeFrame(film, exposure, frame, samples, timing) {
  for (const time of sampleTimes(frame, samples, timing)) {
    exposure.add(decodePng(await film.capture(time)));
  }
  return exposure.develop(frame, timing.grain);
}

function runFfmpeg(args, { input } = {}) {
  const ffmpeg = spawn('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', ...args], {
    stdio: [input ? 'pipe' : 'ignore', 'inherit', 'inherit'],
  });
  const finished = once(ffmpeg, 'exit').then(([code]) => {
    if (code !== 0) throw new Error(`ffmpeg exited with code ${code}`);
  });
  return { ffmpeg, finished };
}

async function runWorker() {
  const [job] = await once(process, 'message');
  const film = await openFilm(job);
  const exposure = new Exposure(WIDTH * job.scale, HEIGHT * job.scale);

  for (const chunk of job.chunks) {
    const { ffmpeg, finished } = runFfmpeg(
      [
        '-f', 'rawvideo',
        '-pix_fmt', 'rgb24',
        '-video_size', `${exposure.width}x${exposure.height}`,
        '-framerate', String(job.fps),
        '-i', 'pipe:0',
        '-c:v', 'libx264rgb',
        '-preset', 'ultrafast',
        '-qp', '0',
        chunk.file,
      ],
      { input: true },
    );
    for (let frame = chunk.from; frame < chunk.to; frame++) {
      const rgb = await exposeFrame(film, exposure, frame, job.samples[frame], job);
      if (!ffmpeg.stdin.write(rgb)) await once(ffmpeg.stdin, 'drain');
      process.send({ frame });
    }
    ffmpeg.stdin.end();
    await finished;
  }

  await film.close();
  process.exit(0);
}

// Maps the film's own estimate of how fast a frame moves to an exposure count,
// so whip pans get a smooth blur and held frames are not exposed dozens of times.
function exposuresFor(motion, maxSamples) {
  if (maxSamples <= 1) return 1;
  if (motion >= 2) return maxSamples;
  if (motion >= 1) return Math.max(2, maxSamples >> 2);
  return Math.max(2, maxSamples >> 3);
}

async function renderVideo(options, url) {
  const { fps, scale, maxSamples, shutter, workers } = options;
  const probe = await openFilm({ url, scale: 1 });
  const duration = await probe.evaluate('FILM.duration');
  const firstFrame = Math.round((options.from ?? 0) * fps);
  const lastFrame = Math.round((options.to ?? duration) * fps);
  const motion = await probe.evaluate(
    `Array.from({ length: ${lastFrame} }, (_, frame) => FILM.motion(frame / ${fps}))`,
  );
  await probe.close();

  const samples = motion.map((level) => exposuresFor(level, maxSamples));
  const exposures = samples.slice(firstFrame).reduce((total, count) => total + count, 0);

  const chunkDir = path.join(distDir, 'chunks');
  rmSync(chunkDir, { recursive: true, force: true });
  mkdirSync(chunkDir, { recursive: true });

  const chunkFrames = Math.max(6, Math.round(fps / 4));
  const chunks = [];
  for (let from = firstFrame; from < lastFrame; from += chunkFrames) {
    const file = path.join(chunkDir, `chunk_${String(chunks.length).padStart(4, '0')}.mkv`);
    chunks.push({ from, to: Math.min(lastFrame, from + chunkFrames), file });
  }

  const total = lastFrame - firstFrame;
  const started = Date.now();
  let done = 0;
  console.log(
    `Rendering ${total} frames at ${WIDTH * scale}x${HEIGHT * scale}, ${fps} fps, ` +
      `${exposures} exposures across ${workers} workers`,
  );

  await Promise.all(
    Array.from({ length: Math.min(workers, chunks.length) }, (_, worker) => {
      const child = fork(fileURLToPath(import.meta.url), ['--worker']);
      child.send({
        url,
        scale,
        fps,
        shutter,
        grain: options.grain,
        duration,
        samples,
        chunks: chunks.filter((_, index) => index % workers === worker),
      });
      child.on('message', () => {
        done++;
        if (done % 30 === 0 || done === total) {
          const elapsed = (Date.now() - started) / 1000;
          process.stdout.write(`  ${done}/${total} frames, ${elapsed.toFixed(0)}s\n`);
        }
      });
      return once(child, 'exit').then(([code]) => {
        if (code !== 0) throw new Error(`Render worker ${worker} exited with code ${code}`);
      });
    }),
  );

  const list = path.join(chunkDir, 'chunks.txt');
  writeFileSync(list, chunks.map((chunk) => `file '${chunk.file}'\n`).join(''));
  const master = path.join(distDir, `${options.name}.master.mkv`);
  await runFfmpeg(['-f', 'concat', '-safe', '0', '-i', list, '-c', 'copy', master]).finished;
  rmSync(chunkDir, { recursive: true, force: true });

  await encodeDelivery(options, master);
}

async function encodeDelivery(options, master) {
  const audio = path.join(distDir, 'soundtrack.wav');
  const withAudio = !options.silent && existsSync(audio);
  const output = path.join(distDir, `${options.name}.mp4`);
  const filters = [];
  if (options.height) filters.push(`scale=-2:${options.height}:flags=lanczos`);
  // Screenshots are full-range sRGB. Tagging the sRGB transfer rather than the
  // BT.709 one stops colour-managed players from lifting the blacks.
  filters.push(
    'scale=out_color_matrix=bt709:out_range=limited',
    'format=yuv420p',
    'setparams=range=limited:color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709',
  );

  await runFfmpeg([
    '-i', master,
    // A partial render starts mid-film, so the soundtrack has to start there too.
    ...(withAudio ? ['-ss', String(options.from ?? 0), '-i', audio] : []),
    '-vf', filters.join(','),
    '-c:v', 'libx264',
    '-preset', 'slow',
    '-crf', String(options.crf),
    '-profile:v', 'high',
    '-pix_fmt', 'yuv420p',
    ...(withAudio ? ['-c:a', 'aac', '-b:a', '256k', '-shortest'] : ['-an']),
    '-movflags', '+faststart',
    output,
  ]).finished;

  console.log(`Wrote ${path.relative(process.cwd(), output)}${withAudio ? '' : ' (no soundtrack)'}`);
}

async function renderStills(options, url, times) {
  const { fps, scale, maxSamples, shutter } = options;
  const dir = path.join(distDir, 'stills');
  mkdirSync(dir, { recursive: true });
  const film = await openFilm({ url, scale });
  const duration = await film.evaluate('FILM.duration');
  const exposure = new Exposure(WIDTH * scale, HEIGHT * scale);
  const files = [];

  for (const time of times) {
    const frame = Math.round(time * fps);
    const motion = await film.evaluate(`FILM.motion(${frame / fps})`);
    const samples = options.blur ? exposuresFor(motion, maxSamples) : 1;
    const rgb = await exposeFrame(film, exposure, frame, samples, { fps, shutter, duration, grain: options.grain });
    const file = path.join(dir, `${options.prefix ?? 'still'}_${(frame / fps).toFixed(3).padStart(6, '0')}.png`);
    writeFileSync(file, encodePng(rgb, exposure.width, exposure.height));
    files.push(file);
  }

  await film.close();
  return files;
}

async function renderSheet(options, url, spec) {
  const [from, to, count = '12', columns = '4'] = spec.split(':');
  const times = Array.from(
    { length: Number(count) },
    (_, index) => Number(from) + ((Number(to) - Number(from)) * index) / Math.max(1, Number(count) - 1),
  );
  const sheetDir = path.join(distDir, 'stills', 'sheet');
  rmSync(sheetDir, { recursive: true, force: true });
  mkdirSync(sheetDir, { recursive: true });
  const files = await renderStills({ ...options, prefix: 'sheet/tile' }, `${url}&timecode=1`, times);
  const rows = Math.ceil(files.length / Number(columns));
  const output = path.join(distDir, 'stills', `sheet_${from}_${to}.png`);
  await runFfmpeg([
    '-framerate', '1',
    '-pattern_type', 'glob',
    '-i', path.join(sheetDir, 'tile_*.png'),
    '-vf', `scale=${Math.round(1920 / Number(columns))}:-1:flags=lanczos,tile=${columns}x${rows}:padding=4:color=0x333333`,
    '-frames:v', '1',
    output,
  ]).finished;
  console.log(`Wrote ${path.relative(process.cwd(), output)}`);
}

// Seeks to scattered times and checks that the screenshot shows that exact
// time, which is the property the whole renderer depends on.
async function verifySeeking(url) {
  const film = await openFilm({ url: `${url}&probe=1`, scale: 1 });
  const duration = await film.evaluate('FILM.duration');
  let failures = 0;
  for (let index = 0; index < 60; index++) {
    const time = ((index * 7919) % 1000) / 1000 * duration;
    const expected = await film.evaluate(`FILM.probeColumn(${time})`);
    const { pixels, channels } = decodePng(await film.capture(time));
    const at = (column) => (2 * WIDTH + column) * channels;
    const lit = pixels[at(expected)] > 200 && pixels[at(expected) + 1] < 60 && pixels[at(expected) + 2] > 200;
    if (!lit) failures++;
  }
  await film.close();
  if (failures > 0) throw new Error(`${failures} of 60 seeks returned a stale frame`);
  console.log('Seeking is frame-accurate: 60 of 60 probes matched');
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.worker) return runWorker();

  for (const font of REQUIRED_FONTS) {
    if (!existsSync(path.join(repoRoot, font))) {
      throw new Error(`Missing font ${font}. Run "pnpm install" in website/ to fetch Poppins.`);
    }
  }
  mkdirSync(distDir, { recursive: true });

  const options = {
    fps: Number(args.fps ?? 60),
    scale: Number(args.scale ?? 1),
    maxSamples: Number(args.samples ?? 32),
    shutter: Number(args.shutter ?? 0.75),
    grain: Number(args.grain ?? 1.4),
    workers: Number(args.workers ?? Math.max(1, Math.min(8, availableParallelism() - 2))),
    crf: Number(args.crf ?? 14),
    from: args.from === undefined ? undefined : Number(args.from),
    to: args.to === undefined ? undefined : Number(args.to),
    height: args.height === undefined ? undefined : Number(args.height),
    name: args.name ?? 'reviu-15s',
    silent: Boolean(args.silent),
    blur: Boolean(args.blur),
  };

  if (args.encode) {
    await encodeDelivery(options, path.join(distDir, `${options.name}.master.mkv`));
    return;
  }

  const { server, port } = await startServer();
  const url = `http://127.0.0.1:${port}/promo/film.html?render=1`;

  try {
    if (args.preview) {
      console.log(`Preview at http://127.0.0.1:${port}/promo/film.html (ctrl-c to stop)`);
      await once(process, 'SIGINT');
    } else if (args.verify) {
      await verifySeeking(url);
    } else if (args.still) {
      const files = await renderStills(options, url, String(args.still).split(',').map(Number));
      for (const file of files) console.log(`Wrote ${path.relative(process.cwd(), file)}`);
    } else if (args.sheet) {
      await renderSheet(options, url, String(args.sheet));
    } else {
      await renderVideo(options, url);
    }
  } finally {
    server.close();
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
