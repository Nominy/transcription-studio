// Renders one parity case in wavesurfer (upstream 7.12.1, closest to the build Babel ships), the
// way Babel's editor sets a lane up (options as in the editor's `WaveSurfer.create` call), and records the
// outline every canvas draws. `renderCase(spec, studio)` with `studio` set draws Studio's
// outline (from the Rust side) through the same wavesurfer DOM via `renderFunction`, so
// pixel differences between the two renders are geometry only.
(function () {
  // Record path ops per canvas (wavesurfer draws one beginPath ... fill per canvas).
  const proto = CanvasRenderingContext2D.prototype;
  for (const name of ['beginPath', 'moveTo', 'lineTo', 'fill']) {
    const original = proto[name];
    proto[name] = function (...args) {
      (this.canvas.__ops ||= []).push([name, ...args]);
      return original.apply(this, args);
    };
  }

  // Babel's streaming WAV decode (`nv`): PCM16 / 32768 or Float32, one Float32Array per channel.
  function parseWav(buffer) {
    const view = new DataView(buffer);
    const tag = (offset) => String.fromCharCode(...new Uint8Array(buffer, offset, 4));
    if (tag(0) !== 'RIFF' || tag(8) !== 'WAVE') throw new Error('not a RIFF/WAVE file');
    let offset = 12;
    let format = null;
    while (offset + 8 <= buffer.byteLength) {
      const id = tag(offset);
      const size = view.getUint32(offset + 4, true);
      if (id === 'fmt ') {
        const code = view.getUint16(offset + 8, true);
        format = {
          channels: view.getUint16(offset + 10, true),
          sampleRate: view.getUint32(offset + 12, true),
          bits: view.getUint16(offset + 22, true),
          float: code === 3,
        };
      } else if (id === 'data') {
        const bytesPerSample = format.bits / 8;
        const frames = Math.floor(size / (bytesPerSample * format.channels));
        const channels = Array.from({ length: format.channels }, () => new Float32Array(frames));
        for (let frame = 0; frame < frames; frame++) {
          for (let channel = 0; channel < format.channels; channel++) {
            const at = offset + 8 + (frame * format.channels + channel) * bytesPerSample;
            channels[channel][frame] = format.float
              ? view.getFloat32(at, true)
              : view.getInt16(at, true) / 32768;
          }
        }
        return { sampleRate: format.sampleRate, frames, channels };
      }
      offset += 8 + size + (size % 2);
    }
    throw new Error('no data chunk');
  }

  const nextFrame = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));
  const wavCache = new Map();
  let current = null;

  async function loadWav(url) {
    if (!wavCache.has(url)) {
      const response = await fetch(url);
      wavCache.set(url, parseWav(await response.arrayBuffer()));
    }
    return wavCache.get(url);
  }

  function outline(ops) {
    const subpaths = [];
    for (const [name, x, y] of ops) {
      if (name === 'beginPath') subpaths.length = 0;
      else if (name === 'moveTo') subpaths.push([[x, y]]);
      else if (name === 'lineTo') subpaths[subpaths.length - 1].push([x, y]);
    }
    return subpaths;
  }

  window.renderCase = async function renderCase(spec, studio) {
    if (current) current.destroy();
    const lane = document.getElementById('lane');
    lane.style.width = `${spec.clientWidth}px`;
    lane.style.height = `${spec.height}px`;
    lane.style.background = spec.background;
    const underlay = document.getElementById('underlay');
    underlay.innerHTML = '';
    for (const rect of spec.underlay || []) {
      const div = document.createElement('div');
      Object.assign(div.style, {
        position: 'absolute',
        top: '0',
        bottom: '0',
        left: `${rect.left}px`,
        width: `${rect.width}px`,
        background: rect.color,
      });
      underlay.appendChild(div);
    }
    const container = document.getElementById('wave');
    container.innerHTML = '';
    container.style.height = `${spec.height}px`;

    const wav = await loadWav(spec.audioUrl);
    const duration = wav.frames / wav.sampleRate;
    const signal = new window.BabelDisplaySignal(wav.sampleRate, duration);

    const studioCanvases = studio ? new Map(studio.canvases.map((c) => [c.left, c])) : null;
    const ws = window.WaveSurfer.create({
      container,
      audioRate: 1,
      height: spec.height,
      barHeight: spec.barHeight,
      waveColor: spec.waveColor,
      progressColor: spec.progressColor,
      cursorColor: '#000',
      cursorWidth: 1,
      normalize: false,
      autoCenter: true,
      fillParent: false,
      hideScrollbar: true,
      minPxPerSec: 200,
      dragToSeek: false,
      ...(studioCanvases && {
        renderFunction(_channels, ctx) {
          const canvas = studioCanvases.get(parseFloat(ctx.canvas.style.left));
          if (!canvas) return;
          ctx.beginPath();
          for (const subpath of [canvas.top, canvas.bottom]) {
            ctx.moveTo(subpath[0][0], subpath[0][1]);
            for (let i = 1; i < subpath.length; i++) ctx.lineTo(subpath[i][0], subpath[i][1]);
          }
          ctx.fill();
          ctx.closePath();
        },
      }),
    });
    current = ws;
    // As Babel's editor: load the (still empty) streaming buffer, let the stream fill it,
    // re-render (`setOptions({})` on stream progress).
    await ws.load('', [signal.samples], duration);
    signal.push(wav.channels, 0, wav.frames);
    ws.setOptions({});
    ws.zoom(spec.pxPerSec);
    ws.setTime(spec.progress);
    ws.setScroll(spec.scroll);
    for (let i = 0; i < 4; i++) await nextFrame();

    const scroller = ws.getWrapper().parentElement;
    const canvases = [...ws.getWrapper().querySelector('.canvases').querySelectorAll('canvas')].map(
      (canvas) => {
        const [top, bottom] = outline(canvas.__ops || []);
        return {
          left: parseFloat(canvas.style.left),
          width: parseFloat(canvas.style.width),
          height: parseFloat(canvas.style.height),
          deviceWidth: canvas.width,
          deviceHeight: canvas.height,
          top,
          bottom,
        };
      },
    );
    canvases.sort((a, b) => a.left - b.left);
    return {
      duration,
      sampleRate: wav.sampleRate,
      channels: wav.channels.length,
      signalLength: signal.samples.length,
      scrollLeft: scroller.scrollLeft,
      scrollWidth: scroller.scrollWidth,
      clientWidth: scroller.clientWidth,
      pixelRatio: window.devicePixelRatio,
      canvases,
    };
  };

  /** Babel's display signal of a WAV, as little-endian f32 bytes (base64). */
  window.displaySignal = async function displaySignal(url) {
    const wav = await loadWav(url);
    const signal = new window.BabelDisplaySignal(wav.sampleRate, wav.frames / wav.sampleRate);
    signal.push(wav.channels, 0, wav.frames);
    const bytes = new Uint8Array(signal.samples.buffer);
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  };
})();
