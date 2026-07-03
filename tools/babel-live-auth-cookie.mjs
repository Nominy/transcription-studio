#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const DEFAULT_CDP_ENDPOINT = "http://127.0.0.1:9222";
const DEFAULT_BABEL_URL = "https://dashboard.babel.audio";

const args = parseArgs(process.argv.slice(2));
const cdpEndpoint =
  args.endpoint || process.env.STUDIO_BABEL_CDP_ENDPOINT || DEFAULT_CDP_ENDPOINT;
const babelUrl =
  args.url || process.env.STUDIO_BABEL_SERVICE_BASE_URL || DEFAULT_BABEL_URL;

try {
  const cookieHeader = await readCookieHeader(cdpEndpoint, babelUrl);
  if (!cookieHeader) {
    throw new Error(`No cookies visible for ${babelUrl}`);
  }
  process.stdout.write(`${cookieHeader}\n`);
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`babel-live-auth-cookie: ${message}\n`);
  process.exitCode = 1;
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--endpoint") {
      parsed.endpoint = argv[++index];
    } else if (arg.startsWith("--endpoint=")) {
      parsed.endpoint = arg.slice("--endpoint=".length);
    } else if (arg === "--url") {
      parsed.url = argv[++index];
    } else if (arg.startsWith("--url=")) {
      parsed.url = arg.slice("--url=".length);
    }
  }
  return parsed;
}

async function readCookieHeader(endpoint, url) {
  const target = await findDebugTarget(endpoint, url);
  if (target.browserWebSocketDebuggerUrl) {
    return readCookieHeaderFromBrowserTarget(
      target.browserWebSocketDebuggerUrl,
      url,
    );
  }

  const client = await connectCdp(target.webSocketDebuggerUrl);
  try {
    await client.send("Network.enable").catch(() => undefined);
    const result = await client.send("Network.getCookies", { urls: [url] });
    const cookies = Array.isArray(result.cookies) ? result.cookies : [];
    return cookies
      .filter((cookie) => cookie && cookie.name && cookie.value !== undefined)
      .map((cookie) => `${cookie.name}=${cookie.value}`)
      .join("; ");
  } finally {
    client.close();
  }
}

async function findDebugTarget(endpoint, url) {
  try {
    const targets = await fetchJson(
      new URL("/json/list", normalizeEndpoint(endpoint)),
    );
    const fallback = choosePageTarget(targets, url);
    if (fallback) {
      return fallback;
    }
  } catch {
    // Chrome can expose only DevToolsActivePort/browser WebSocket on some starts.
  }

  const browserWebSocketDebuggerUrl =
    (await browserWebSocketFromVersion(endpoint)) ||
    browserWebSocketFromActivePort(endpoint);
  if (browserWebSocketDebuggerUrl) {
    return { browserWebSocketDebuggerUrl };
  }

  throw new Error(
    `No debuggable Chrome target found at ${endpoint}; start Chrome with --remote-debugging-port=9222`,
  );
}

function choosePageTarget(targets, url) {
  const wantedOrigin = new URL(url).origin;
  const pages = targets.filter(
    (target) => target.type === "page" && target.webSocketDebuggerUrl,
  );
  const matching = pages.find((target) => {
    try {
      return new URL(target.url).origin === wantedOrigin;
    } catch {
      return false;
    }
  });
  return matching || pages[0];
}

async function browserWebSocketFromVersion(endpoint) {
  try {
    const version = await fetchJson(
      new URL("/json/version", normalizeEndpoint(endpoint)),
    );
    return typeof version.webSocketDebuggerUrl === "string"
      ? version.webSocketDebuggerUrl
      : "";
  } catch {
    return "";
  }
}

function browserWebSocketFromActivePort(endpoint) {
  const activePort = readDevToolsActivePort();
  if (!activePort) {
    return "";
  }
  const endpointUrl = normalizeEndpoint(endpoint);
  endpointUrl.port = activePort.port;
  endpointUrl.protocol = endpointUrl.protocol === "https:" ? "wss:" : "ws:";
  endpointUrl.pathname = activePort.path;
  endpointUrl.search = "";
  endpointUrl.hash = "";
  return endpointUrl.toString();
}

function readDevToolsActivePort() {
  const userDataDir =
    process.env.STUDIO_BABEL_CHROME_USER_DATA_DIR ||
    (process.env.LOCALAPPDATA
      ? `${process.env.LOCALAPPDATA}\\Google\\Chrome\\User Data`
      : "");
  if (!userDataDir) {
    return null;
  }

  try {
    const lines = fs
      .readFileSync(path.join(userDataDir, "DevToolsActivePort"), "utf8")
      .trim()
      .split(/\r?\n/);
    if (lines.length >= 2) {
      return { port: lines[0], path: lines[1] };
    }
  } catch {
    return null;
  }
  return null;
}

async function readCookieHeaderFromBrowserTarget(browserWebSocketDebuggerUrl, url) {
  const client = await connectCdp(browserWebSocketDebuggerUrl);
  let sessionId = "";
  try {
    const targets = await client.send("Target.getTargets");
    const target = chooseBrowserPageTarget(targets.targetInfos || [], url);
    if (!target) {
      throw new Error("No page target is available in Chrome");
    }
    const attached = await client.send("Target.attachToTarget", {
      targetId: target.targetId,
      flatten: true,
    });
    sessionId = attached.sessionId;
    await client.send("Network.enable", {}, sessionId).catch(() => undefined);
    const result = await client.send("Network.getCookies", { urls: [url] }, sessionId);
    const cookies = Array.isArray(result.cookies) ? result.cookies : [];
    return cookies
      .filter((cookie) => cookie && cookie.name && cookie.value !== undefined)
      .map((cookie) => `${cookie.name}=${cookie.value}`)
      .join("; ");
  } finally {
    if (sessionId) {
      await client
        .send("Target.detachFromTarget", { sessionId })
        .catch(() => undefined);
    }
    client.close();
  }
}

function chooseBrowserPageTarget(targets, url) {
  const wantedOrigin = new URL(url).origin;
  const pages = targets.filter((target) => target.type === "page");
  return (
    pages.find((target) => {
      try {
        return new URL(target.url).origin === wantedOrigin;
      } catch {
        return false;
      }
    }) || pages[0]
  );
}

function normalizeEndpoint(endpoint) {
  const normalized = new URL(endpoint);
  if (!normalized.pathname.endsWith("/")) {
    normalized.pathname = `${normalized.pathname}/`;
  }
  return normalized;
}

async function fetchJson(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(5000) });
  if (!response.ok) {
    throw new Error(`Chrome DevTools endpoint returned HTTP ${response.status}`);
  }
  return response.json();
}

function connectCdp(webSocketUrl) {
  if (typeof WebSocket !== "function") {
    throw new Error("This Node.js runtime does not provide WebSocket");
  }

  const socket = new WebSocket(webSocketUrl);
  let nextId = 1;
  const pending = new Map();

  socket.addEventListener("message", (event) => {
    let payload = event.data;
    if (payload instanceof ArrayBuffer) {
      payload = Buffer.from(payload).toString("utf8");
    } else if (ArrayBuffer.isView(payload)) {
      payload = Buffer.from(payload.buffer).toString("utf8");
    }

    let message;
    try {
      message = JSON.parse(String(payload));
    } catch {
      return;
    }

    if (!message.id || !pending.has(message.id)) {
      return;
    }
    const { resolve, reject, timeout } = pending.get(message.id);
    clearTimeout(timeout);
    pending.delete(message.id);
    if (message.error) {
      reject(new Error(message.error.message || JSON.stringify(message.error)));
    } else {
      resolve(message.result || {});
    }
  });

  const opened = new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error("Timed out connecting to Chrome DevTools WebSocket")),
      5000,
    );
    socket.addEventListener(
      "open",
      () => {
        clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
    socket.addEventListener(
      "error",
      () => {
        clearTimeout(timeout);
        reject(new Error("Chrome DevTools WebSocket connection failed"));
      },
      { once: true },
    );
  });

  return opened.then(() => ({
    send(method, params = {}, sessionId = "") {
      const id = nextId++;
      const payload = JSON.stringify({
        id,
        method,
        params,
        ...(sessionId ? { sessionId } : {}),
      });
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`Timed out waiting for CDP ${method}`));
        }, 5000);
        pending.set(id, { resolve, reject, timeout });
        socket.send(payload);
      });
    },
    close() {
      socket.close();
    },
  }));
}
