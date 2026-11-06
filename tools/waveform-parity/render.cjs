// Headless Chromium side of the waveform parity benchmark (Playwright; resolve it with
// NODE_PATH, see run.ps1). wavesurfer is upstream wavesurfer.js (babel/wavesurfer.js, BSD-3);
// Babel's display-signal downsampler is proprietary and never part of this repository: point
// BABEL_DISPLAY_SIGNAL at a local copy of that editor-chunk excerpt (it defines
// window.BabelDisplaySignal).
//
//   node render.cjs babel    <cases.json> <audioDir> <outDir>   Babel's wavesurfer renders
//   node render.cjs studio   <outDir>                            Studio geometry via wavesurfer
//   node render.cjs fixtures <fixturesDir>                       expected data for Rust tests
//   node render.cjs app      <outDir> <audioDir>                 Babel at the app's views
const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const BABEL_DIR = path.join(__dirname, 'babel');
const DISPLAY_SIGNAL = process.env.BABEL_DISPLAY_SIGNAL;
if (!DISPLAY_SIGNAL || !fs.existsSync(DISPLAY_SIGNAL)) {
  throw new Error('BABEL_DISPLAY_SIGNAL must name a local copy of Babel\'s display-signal downsampler');
}
const ORIGIN = 'http://parity.local';

function log(message) {
  console.log(`[${new Date().toISOString()}] ${message}`);
}

async function openPage(browser, dpr, files) {
  const context = await browser.newContext({
    viewport: { width: 1800, height: 400 },
    deviceScaleFactor: dpr,
  });
  const page = await context.newPage();
  page.on('pageerror', (error) => log(`page error: ${error.message}`));
  await page.route(`${ORIGIN}/**`, (route) => {
    const name = decodeURIComponent(new URL(route.request().url()).pathname.slice(1));
    const file = name.startsWith('audio/')
      ? path.join(files.audioDir, name.slice('audio/'.length))
      : name === 'display-signal.js' ? DISPLAY_SIGNAL : path.join(BABEL_DIR, name);
    if (!fs.existsSync(file)) return route.fulfill({ status: 404 });
    route.fulfill({ status: 200, body: fs.readFileSync(file) });
  });
  await page.goto(`${ORIGIN}/page.html`);
  await page.waitForFunction(() => typeof window.renderCase === 'function');
  return { context, page };
}

function resolveCases(file) {
  const { defaults, cases } = JSON.parse(fs.readFileSync(file, 'utf8'));
  return cases.map((spec) => ({ ...defaults, ...spec }));
}

async function withPages(browser, audioDir, specs, run) {
  const pages = new Map();
  try {
    for (const spec of specs) {
      if (!pages.has(spec.dpr)) pages.set(spec.dpr, await openPage(browser, spec.dpr, { audioDir }));
      await run(pages.get(spec.dpr).page, spec);
    }
  } finally {
    for (const { context } of pages.values()) await context.close();
  }
}

async function shoot(page, spec, file) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  await page.locator('#lane').screenshot({ path: file, animations: 'disabled' });
}

async function babel(casesFile, audioDir, outDir) {
  const specs = resolveCases(casesFile);
  const browser = await chromium.launch();
  const resolved = [];
  const signals = new Set();
  try {
    await withPages(browser, audioDir, specs, async (page, spec) => {
      const started = Date.now();
      spec.audioUrl = `${ORIGIN}/audio/${spec.audio}`;
      if (spec.fit) {
        const duration = await page.evaluate(async (url) => {
          const wav = await (await fetch(url)).arrayBuffer();
          const view = new DataView(wav);
          // Duration from the header: data bytes / block align / rate (PCM16 only here).
          let offset = 12;
          let rate = 0;
          let align = 0;
          while (offset + 8 <= wav.byteLength) {
            const id = String.fromCharCode(...new Uint8Array(wav, offset, 4));
            const size = view.getUint32(offset + 4, true);
            if (id === 'fmt ') {
              rate = view.getUint32(offset + 12, true);
              align = view.getUint16(offset + 20, true);
            }
            if (id === 'data') return size / align / rate;
            offset += 8 + size + (size % 2);
          }
          throw new Error('no data chunk');
        }, spec.audioUrl);
        spec.pxPerSec = spec.clientWidth / duration;
      }
      const result = await page.evaluate((s) => window.renderCase(s), spec);
      spec.scroll = result.scrollLeft;
      spec.duration = result.duration;
      await shoot(page, spec, path.join(outDir, 'babel', `${spec.id}.png`));
      fs.writeFileSync(path.join(outDir, 'babel', `${spec.id}.json`), JSON.stringify(result));
      resolved.push(spec);
      if (!signals.has(spec.audio)) {
        signals.add(spec.audio);
        const base64 = await page.evaluate((url) => window.displaySignal(url), spec.audioUrl);
        fs.writeFileSync(
          path.join(outDir, 'babel', `signal-${spec.audio}.f32`),
          Buffer.from(base64, 'base64'),
        );
      }
      log(`babel ${spec.id}: ${result.canvases.length} canvases, scroll ${result.scrollLeft}, ${Date.now() - started} ms`);
    });
  } finally {
    await browser.close();
  }
  fs.writeFileSync(path.join(outDir, 'resolved-cases.json'), JSON.stringify(resolved, null, 2));
}

async function studio(outDir) {
  const specs = JSON.parse(fs.readFileSync(path.join(outDir, 'resolved-cases.json'), 'utf8'));
  const audioDir = path.join(outDir, 'audio');
  const browser = await chromium.launch();
  try {
    await withPages(browser, audioDir, specs, async (page, spec) => {
      const geometry = JSON.parse(
        fs.readFileSync(path.join(outDir, 'studio', `${spec.id}.json`), 'utf8'),
      );
      await page.evaluate(([s, g]) => window.renderCase(s, g), [spec, geometry]);
      await shoot(page, spec, path.join(outDir, 'studio-geometry', `${spec.id}.png`));
      log(`studio geometry ${spec.id}: ${geometry.canvases.length} canvases`);
    });
  } finally {
    await browser.close();
  }
}

// Expected display signals and canvas outlines of the short fixtures, from Babel's code.
async function fixtures(fixturesDir) {
  const browser = await chromium.launch();
  const views = [
    { clientWidth: 300, height: 150, dpr: 1, pxPerSec: 2000, scroll: 0, barHeight: 20 },
    { clientWidth: 160, height: 96, dpr: 1.5, pxPerSec: 1000, scroll: 30, barHeight: 2.5 },
    { clientWidth: 120, height: 48, dpr: 2, pxPerSec: 700, scroll: 0, barHeight: 0.3 },
  ];
  try {
    const wavs = fs.readdirSync(fixturesDir).filter((name) => name.endsWith('.wav'));
    const specs = wavs.flatMap((wav) =>
      views.map((view, index) => ({
        ...view,
        id: `${wav}#${index}`,
        audio: wav,
        progress: 0,
        waveColor: '#707070',
        progressColor: '#64B5F6',
        background: '#ffffff',
      })),
    );
    const expected = new Map();
    await withPages(browser, fixturesDir, specs, async (page, spec) => {
      spec.audioUrl = `${ORIGIN}/audio/${spec.audio}`;
      const result = await page.evaluate((s) => window.renderCase(s), spec);
      if (!expected.has(spec.audio)) {
        const base64 = await page.evaluate((url) => window.displaySignal(url), spec.audioUrl);
        const bytes = Buffer.from(base64, 'base64');
        const bits = [];
        for (let i = 0; i < bytes.length; i += 4) bits.push(bytes.readUInt32LE(i));
        expected.set(spec.audio, { signal_bits: bits, views: [] });
      }
      expected.get(spec.audio).views.push({
        view: {
          min_px_per_sec: spec.pxPerSec,
          client_width: spec.clientWidth,
          height: spec.height,
          pixel_ratio: spec.dpr,
          scroll_left: result.scrollLeft,
          bar_height: spec.barHeight,
        },
        canvases: result.canvases.map((c) => ({
          left: c.left,
          device_width: c.deviceWidth,
          device_height: c.deviceHeight,
          top: c.top,
          bottom: c.bottom,
        })),
      });
    });
    for (const [wav, data] of expected) {
      const file = path.join(fixturesDir, wav.replace(/\.wav$/, '.json'));
      fs.writeFileSync(file, JSON.stringify(data));
      log(`fixture ${file}`);
    }
  } finally {
    await browser.close();
  }
}

// Babel at the views captured from the running app (app_compare.py prepare).
async function app(outDir, audioDir) {
  const specs = JSON.parse(fs.readFileSync(path.join(outDir, 'app', 'babel-cases.json'), 'utf8'));
  const browser = await chromium.launch();
  try {
    await withPages(browser, audioDir, specs, async (page, spec) => {
      spec.audioUrl = `${ORIGIN}/audio/${spec.audio}`;
      const result = await page.evaluate((s) => window.renderCase(s), spec);
      await shoot(page, spec, path.join(outDir, 'app', `${spec.id}-babel.png`));
      log(`babel app view ${spec.id}: ${result.canvases.length} canvases, scroll ${result.scrollLeft}`);
    });
  } finally {
    await browser.close();
  }
}

const [mode, ...args] = process.argv.slice(2);
const run = { babel, studio, fixtures, app }[mode];
if (!run) {
  console.error('usage: node render.cjs babel|studio|fixtures|app ...');
  process.exit(2);
}
run(...args).catch((error) => {
  console.error(error);
  process.exit(1);
});
