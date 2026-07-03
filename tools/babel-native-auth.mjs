#!/usr/bin/env node

import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { createInterface } from "node:readline/promises";

const DEFAULT_BABEL_URL = "https://dashboard.babel.audio";
const DEFAULT_CLERK_URL = "https://clerk.babel.audio";
const CLERK_API_VERSION = "2025-11-10";
const CLERK_JS_VERSION = "5.127.0";
const CLERK_INSTANCE_SUFFIX = "v1aVht_F";

export async function runCli(argv, dependencies = {}) {
  const { command, args } = parseCommand(argv);
  if (!command || command === "help" || args.help) {
    process.stdout.write(helpText());
    return;
  }

  const options = optionsFromArgs(args, dependencies);
  if (command === "start-login") {
    const identifier = args.identifier || process.env.STUDIO_BABEL_AUTH_IDENTIFIER;
    if (!identifier) {
      throw new Error("start-login requires --identifier or STUDIO_BABEL_AUTH_IDENTIFIER");
    }
    const result = await startPhoneCodeLogin({
      ...options,
      identifier,
      reset: args.reset === true,
    });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return;
  }

  if (command === "verify-login") {
    const result = await verifyPhoneCodeLogin({
      ...options,
      code: args.code,
    });
    process.stdout.write(`${JSON.stringify(redactedLoginSummary(result), null, 2)}\n`);
    return;
  }

  if (command === "login") {
    const identifier = args.identifier || process.env.STUDIO_BABEL_AUTH_IDENTIFIER;
    if (!identifier) {
      throw new Error("login requires --identifier or STUDIO_BABEL_AUTH_IDENTIFIER");
    }
    const result = await loginWithPhoneCode({
      ...options,
      identifier,
      code: args.code,
      reset: args.reset === true,
    });
    process.stdout.write(`${JSON.stringify(redactedLoginSummary(result), null, 2)}\n`);
    return;
  }

  if (command === "cookie") {
    const result = await mintBabelSessionCookie(options);
    if (args.json) {
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    } else {
      process.stdout.write(`${result.cookie}\n`);
    }
    return;
  }

  if (command === "status") {
    const result = await readNativeAuthStatus(options);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return;
  }

  throw new Error(`unknown command: ${command}`);
}

export async function loginWithPhoneCode({
  identifier,
  code,
  reset = false,
  storePath = defaultStorePath(),
  babelUrl = DEFAULT_BABEL_URL,
  clerkUrl = DEFAULT_CLERK_URL,
  fetchImpl = fetch,
  promptImpl = promptForCode,
  now = () => Date.now(),
} = {}) {
  await startPhoneCodeLogin({
    identifier,
    reset,
    storePath,
    babelUrl,
    clerkUrl,
    fetchImpl,
  });
  const verificationCode = code || (await promptImpl("Babel phone verification code: "));
  return verifyPhoneCodeLogin({
    code: verificationCode,
    storePath,
    babelUrl,
    clerkUrl,
    fetchImpl,
    now,
  });
}

export async function startPhoneCodeLogin({
  identifier,
  reset = false,
  storePath = defaultStorePath(),
  babelUrl = DEFAULT_BABEL_URL,
  clerkUrl = DEFAULT_CLERK_URL,
  fetchImpl = fetch,
} = {}) {
  if (!identifier) {
    throw new Error("phone-code login requires an identifier");
  }
  const jar = reset ? new CookieJar() : await CookieJar.load(storePath);
  const client = new ClerkFrontendClient({ jar, babelUrl, clerkUrl, fetchImpl });
  const signIn = await client.createSignIn(identifier);
  const factor = choosePhoneCodeFactor(signIn);
  await client.prepareFirstFactor(signIn.id, factor);
  jar.pendingSignIn = {
    id: signIn.id,
    phoneNumberId: factor.phone_number_id,
    createdAt: new Date().toISOString(),
  };
  await jar.save(storePath);

  return {
    storePath,
    pending: true,
    signInId: signIn.id,
    factor: "phone_code",
    readyForCode: true,
  };
}

export async function verifyPhoneCodeLogin({
  code,
  storePath = defaultStorePath(),
  babelUrl = DEFAULT_BABEL_URL,
  clerkUrl = DEFAULT_CLERK_URL,
  fetchImpl = fetch,
  now = () => Date.now(),
} = {}) {
  if (!String(code || "").trim()) {
    throw new Error("phone verification code is empty");
  }
  const jar = await CookieJar.load(storePath);
  const pending = jar.pendingSignIn;
  if (!pending || !pending.id) {
    throw new Error("native Babel auth store has no pending phone-code login; run `node tools/babel-native-auth.mjs start-login --identifier <phone>`");
  }
  const client = new ClerkFrontendClient({ jar, babelUrl, clerkUrl, fetchImpl });
  const attempted = await client.attemptFirstFactor(pending.id, String(code).trim());
  const sessionId = activeSessionId(attempted);
  if (!sessionId) {
    throw new Error(`phone-code sign-in did not create an active session; status=${attempted.status || "unknown"}`);
  }

  const token = await client.createSessionToken(sessionId);
  const cookie = babelBackendCookieHeader(token.jwt, { now });
  jar.pendingSignIn = null;
  await jar.save(storePath);

  return {
    storePath,
    sessionId,
    status: attempted.status || "complete",
    expiresAt: jwtExpiryIso(token.jwt),
    cookie,
  };
}

export async function mintBabelSessionCookie({
  storePath = defaultStorePath(),
  babelUrl = DEFAULT_BABEL_URL,
  clerkUrl = DEFAULT_CLERK_URL,
  fetchImpl = fetch,
  now = () => Date.now(),
} = {}) {
  const jar = await CookieJar.load(storePath);
  const client = new ClerkFrontendClient({ jar, babelUrl, clerkUrl, fetchImpl });
  const clerkClient = await client.readClient();
  const sessionId = activeSessionId(clerkClient);
  if (!sessionId) {
    if (jar.pendingSignIn?.id) {
      throw new Error("native Babel auth store has a pending phone-code login; run `node tools/babel-native-auth.mjs verify-login --code <code>`");
    }
    throw new Error("native Babel auth store has no active Clerk session; run `node tools/babel-native-auth.mjs login --identifier <phone>`");
  }

  const token = await client.createSessionToken(sessionId);
  await jar.save(storePath);
  return {
    storePath,
    sessionId,
    expiresAt: jwtExpiryIso(token.jwt),
    cookie: babelBackendCookieHeader(token.jwt, { now }),
  };
}

export async function readNativeAuthStatus({
  storePath = defaultStorePath(),
  babelUrl = DEFAULT_BABEL_URL,
  clerkUrl = DEFAULT_CLERK_URL,
  fetchImpl = fetch,
} = {}) {
  const jar = await CookieJar.load(storePath);
  if (jar.cookies.length === 0) {
    return {
      storePath,
      configured: false,
      active: false,
      pendingLogin: Boolean(jar.pendingSignIn),
      pendingSignInId: jar.pendingSignIn?.id || "",
      cookieCount: 0,
    };
  }

  const client = new ClerkFrontendClient({ jar, babelUrl, clerkUrl, fetchImpl });
  const clerkClient = await client.readClient();
  const sessionId = activeSessionId(clerkClient);
  await jar.save(storePath);
  return {
    storePath,
    configured: true,
    active: Boolean(sessionId),
    pendingLogin: Boolean(jar.pendingSignIn),
    pendingSignInId: jar.pendingSignIn?.id || "",
    sessionId,
    cookieCount: jar.cookies.length,
  };
}

export class ClerkFrontendClient {
  constructor({ jar, babelUrl = DEFAULT_BABEL_URL, clerkUrl = DEFAULT_CLERK_URL, fetchImpl = fetch }) {
    this.jar = jar;
    this.babelUrl = trimTrailingSlash(babelUrl);
    this.clerkUrl = trimTrailingSlash(clerkUrl);
    this.fetchImpl = fetchImpl;
  }

  async createSignIn(identifier) {
    const json = await this.clerkRequest("/v1/client/sign_ins", {
      method: "POST",
      form: { identifier },
    });
    return unwrapClerkResponse(json, ["sign_in", "sign_in_attempt"]);
  }

  async prepareFirstFactor(signInId, factor) {
    return unwrapClerkResponse(
      await this.clerkRequest(`/v1/client/sign_ins/${encodeURIComponent(signInId)}/prepare_first_factor`, {
        method: "POST",
        form: {
          strategy: "phone_code",
          phone_number_id: factor.phone_number_id,
        },
      }),
      ["sign_in", "sign_in_attempt"],
    );
  }

  async attemptFirstFactor(signInId, code) {
    return unwrapClerkResponse(
      await this.clerkRequest(`/v1/client/sign_ins/${encodeURIComponent(signInId)}/attempt_first_factor`, {
        method: "POST",
        form: {
          strategy: "phone_code",
          code,
        },
      }),
      ["sign_in", "sign_in_attempt"],
    );
  }

  async readClient() {
    return unwrapClerkResponse(await this.clerkRequest("/v1/client"), "client");
  }

  async createSessionToken(sessionId) {
    const json = await this.clerkRequest(`/v1/client/sessions/${encodeURIComponent(sessionId)}/tokens`, {
      method: "POST",
      form: { organization_id: "" },
    });
    if (!json || typeof json.jwt !== "string" || !json.jwt) {
      throw new Error("Clerk token response did not include jwt");
    }
    return json;
  }

  async clerkRequest(route, { method = "GET", form } = {}) {
    const url = new URL(route, `${this.clerkUrl}/`);
    url.searchParams.set("__clerk_api_version", CLERK_API_VERSION);
    url.searchParams.set("_clerk_js_version", CLERK_JS_VERSION);

    const headers = {
      accept: "*/*",
      origin: this.babelUrl,
      referer: `${this.babelUrl}/`,
    };
    const cookieHeader = this.jar.headerForUrl(url);
    if (cookieHeader) {
      headers.cookie = cookieHeader;
    }

    let body;
    if (form) {
      headers["content-type"] = "application/x-www-form-urlencoded";
      body = new URLSearchParams(form).toString();
    }

    const response = await this.fetchImpl(url, { method, headers, body });
    this.jar.storeSetCookieHeaders(url, response.headers);
    const text = await response.text();
    let json;
    try {
      json = text ? JSON.parse(text) : null;
    } catch {
      throw new Error(`Clerk ${method} ${url.pathname} returned non-JSON HTTP ${response.status}`);
    }

    if (!response.ok) {
      throw new Error(clerkErrorMessage(json) || `Clerk ${method} ${url.pathname} returned HTTP ${response.status}`);
    }
    return json;
  }
}

export class CookieJar {
  constructor(cookies = [], pendingSignIn = null) {
    this.cookies = cookies;
    this.pendingSignIn = pendingSignIn;
  }

  static async load(storePath) {
    try {
      const raw = await fs.readFile(storePath, "utf8");
      const parsed = JSON.parse(raw);
      return new CookieJar(
        Array.isArray(parsed.cookies) ? parsed.cookies : [],
        parsed.pendingSignIn || null,
      );
    } catch (error) {
      if (error && error.code === "ENOENT") {
        return new CookieJar();
      }
      throw error;
    }
  }

  async save(storePath) {
    await fs.mkdir(path.dirname(storePath), { recursive: true });
    const payload = JSON.stringify(
      {
        version: 1,
        updatedAt: new Date().toISOString(),
        cookies: this.cookies,
        ...(this.pendingSignIn ? { pendingSignIn: this.pendingSignIn } : {}),
      },
      null,
      2,
    );
    await fs.writeFile(storePath, payload, { mode: 0o600 });
    await fs.chmod(storePath, 0o600).catch(() => undefined);
  }

  headerForUrl(urlLike) {
    const url = urlLike instanceof URL ? urlLike : new URL(urlLike);
    const now = Date.now();
    return this.cookies
      .filter((cookie) => cookieApplies(cookie, url, now))
      .map((cookie) => `${cookie.name}=${cookie.value}`)
      .join("; ");
  }

  storeSetCookieHeaders(urlLike, headers) {
    for (const header of setCookieHeaders(headers)) {
      const cookie = parseSetCookie(header, urlLike);
      if (!cookie) {
        continue;
      }
      this.upsert(cookie);
    }
  }

  upsert(cookie) {
    this.cookies = this.cookies.filter(
      (existing) =>
        existing.name !== cookie.name ||
        existing.domain !== cookie.domain ||
        existing.path !== cookie.path,
    );
    if (cookie.expiresAt && cookie.expiresAt <= Date.now()) {
      return;
    }
    this.cookies.push(cookie);
  }
}

export function babelBackendCookieHeader(jwt, { now = () => Date.now() } = {}) {
  if (!jwt || typeof jwt !== "string") {
    throw new Error("cannot build Babel cookie header without a Clerk session jwt");
  }
  const clientUat = String(Math.floor(now() / 1000));
  return [
    `__session=${jwt}`,
    `__session_${CLERK_INSTANCE_SUFFIX}=${jwt}`,
    `__client_uat=${clientUat}`,
    `__client_uat_${CLERK_INSTANCE_SUFFIX}=${clientUat}`,
  ].join("; ");
}

export function choosePhoneCodeFactor(signIn) {
  const factors =
    signIn.supported_first_factors ||
    signIn.supportedFirstFactors ||
    signIn.first_factors ||
    [];
  const factor = factors.find((candidate) => candidate && candidate.strategy === "phone_code");
  if (!factor || !factor.phone_number_id) {
    const strategies = factors.map((candidate) => candidate && candidate.strategy).filter(Boolean);
    throw new Error(`phone_code is not an available first factor; available=${strategies.join(",") || "none"}`);
  }
  return factor;
}

export function activeSessionId(value) {
  const source = value && value.response ? value.response : value;
  if (!source || typeof source !== "object") {
    return "";
  }
  if (typeof source.created_session_id === "string" && source.created_session_id) {
    return source.created_session_id;
  }
  if (typeof source.createdSessionId === "string" && source.createdSessionId) {
    return source.createdSessionId;
  }
  if (typeof source.last_active_session_id === "string" && source.last_active_session_id) {
    return source.last_active_session_id;
  }
  if (Array.isArray(source.sessions)) {
    const active = source.sessions.find((session) => session && session.status === "active") || source.sessions[0];
    if (active && typeof active.id === "string") {
      return active.id;
    }
  }
  return "";
}

export function parseSetCookie(header, urlLike) {
  if (!header || typeof header !== "string") {
    return null;
  }
  const url = urlLike instanceof URL ? urlLike : new URL(urlLike);
  const parts = header.split(";").map((part) => part.trim());
  const first = parts.shift();
  const equals = first.indexOf("=");
  if (equals <= 0) {
    return null;
  }

  const cookie = {
    name: first.slice(0, equals),
    value: first.slice(equals + 1),
    domain: url.hostname,
    path: defaultCookiePath(url.pathname),
    secure: false,
    httpOnly: false,
    sameSite: "",
    expiresAt: null,
  };

  for (const part of parts) {
    const index = part.indexOf("=");
    const key = (index >= 0 ? part.slice(0, index) : part).trim().toLowerCase();
    const value = index >= 0 ? part.slice(index + 1).trim() : "";
    if (key === "domain" && value) {
      cookie.domain = value.replace(/^\./, "").toLowerCase();
    } else if (key === "path" && value) {
      cookie.path = value;
    } else if (key === "secure") {
      cookie.secure = true;
    } else if (key === "httponly") {
      cookie.httpOnly = true;
    } else if (key === "samesite") {
      cookie.sameSite = value;
    } else if (key === "expires" && value) {
      const expires = Date.parse(value);
      cookie.expiresAt = Number.isNaN(expires) ? null : expires;
    } else if (key === "max-age" && value) {
      const seconds = Number(value);
      cookie.expiresAt = Number.isFinite(seconds) ? Date.now() + seconds * 1000 : cookie.expiresAt;
    }
  }

  return cookie;
}

function parseCommand(argv) {
  const [command, ...rest] = argv;
  return { command, args: parseArgs(rest) };
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) {
      continue;
    }
    const equals = arg.indexOf("=");
    if (equals >= 0) {
      parsed[arg.slice(2, equals)] = arg.slice(equals + 1);
    } else {
      const key = arg.slice(2);
      const next = argv[index + 1];
      if (!next || next.startsWith("--")) {
        parsed[key] = true;
      } else {
        parsed[key] = next;
        index += 1;
      }
    }
  }
  return parsed;
}

function optionsFromArgs(args, dependencies) {
  return {
    storePath: args.store || process.env.STUDIO_BABEL_AUTH_STORE || defaultStorePath(),
    babelUrl: args.babelUrl || process.env.STUDIO_BABEL_SERVICE_BASE_URL || DEFAULT_BABEL_URL,
    clerkUrl: args.clerkUrl || process.env.STUDIO_BABEL_CLERK_URL || DEFAULT_CLERK_URL,
    ...dependencies,
  };
}

async function promptForCode(prompt) {
  const input = createInterface({ input: process.stdin, output: process.stderr });
  try {
    return (await input.question(prompt)).trim();
  } finally {
    input.close();
  }
}

function unwrapClerkResponse(json, expectedObject) {
  const response = json && json.response ? json.response : json;
  if (!response || typeof response !== "object") {
    throw new Error(`Clerk response did not include ${expectedObject}`);
  }
  const expectedObjects = Array.isArray(expectedObject) ? expectedObject : [expectedObject];
  if (
    expectedObject &&
    response.object &&
    !expectedObjects.some((candidate) => response.object === candidate)
  ) {
    throw new Error(`Clerk response object was ${response.object}, expected ${expectedObjects.join(" or ")}`);
  }
  return response;
}

function clerkErrorMessage(json) {
  const errors = json && (json.errors || json.response?.errors);
  if (Array.isArray(errors) && errors.length > 0) {
    return errors
      .map((error) => error.long_message || error.message || error.code)
      .filter(Boolean)
      .join("; ");
  }
  return "";
}

function setCookieHeaders(headers) {
  if (!headers) {
    return [];
  }
  if (typeof headers.getSetCookie === "function") {
    return headers.getSetCookie();
  }
  const combined = typeof headers.get === "function" ? headers.get("set-cookie") : "";
  if (!combined) {
    return [];
  }
  return combined.split(/,(?=[^;,]+=)/).map((header) => header.trim());
}

function cookieApplies(cookie, url, now) {
  if (cookie.expiresAt && cookie.expiresAt <= now) {
    return false;
  }
  const host = url.hostname.toLowerCase();
  const domain = String(cookie.domain || "").replace(/^\./, "").toLowerCase();
  if (host !== domain && !host.endsWith(`.${domain}`)) {
    return false;
  }
  const cookiePath = cookie.path || "/";
  return url.pathname.startsWith(cookiePath);
}

function defaultCookiePath(pathname) {
  if (!pathname || pathname[0] !== "/") {
    return "/";
  }
  if (pathname === "/") {
    return "/";
  }
  return pathname.slice(0, pathname.lastIndexOf("/") + 1) || "/";
}

function defaultStorePath() {
  const base =
    process.env.APPDATA ||
    (process.platform === "win32"
      ? path.join(os.homedir(), "AppData", "Roaming")
      : path.join(os.homedir(), ".config"));
  return path.join(base, "TranscriptionStudio", "babel-auth.json");
}

function trimTrailingSlash(value) {
  return String(value || "").trim().replace(/\/+$/, "");
}

function jwtExpiryIso(jwt) {
  const parts = String(jwt || "").split(".");
  if (parts.length < 2) {
    return null;
  }
  try {
    const payload = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"));
    return typeof payload.exp === "number" ? new Date(payload.exp * 1000).toISOString() : null;
  } catch {
    return null;
  }
}

function redactedLoginSummary(result) {
  return {
    storePath: result.storePath,
    sessionId: result.sessionId,
    status: result.status,
    expiresAt: result.expiresAt,
    ready: true,
  };
}

function helpText() {
  return `Usage:
  node tools/babel-native-auth.mjs login --identifier <phone-or-email>
  node tools/babel-native-auth.mjs start-login --identifier <phone-or-email>
  node tools/babel-native-auth.mjs verify-login --code <code>
  node tools/babel-native-auth.mjs cookie
  node tools/babel-native-auth.mjs status

Options:
  --store <path>       Override native auth store path.
  --code <code>        Provide the phone verification code non-interactively.
  --reset              Start login with a fresh native auth store.
  --json               Print JSON for the cookie command.

Environment:
  STUDIO_BABEL_AUTH_IDENTIFIER  Default login identifier.
  STUDIO_BABEL_AUTH_STORE       Default auth store path.
  STUDIO_BABEL_SERVICE_BASE_URL Default Babel URL.
  STUDIO_BABEL_CLERK_URL        Default Clerk frontend URL.
`;
}

function isMainModule(metaUrl) {
  return process.argv[1] && metaUrl === new URL(`file://${process.argv[1].replace(/\\/g, "/")}`).href;
}

if (isMainModule(import.meta.url)) {
  runCli(process.argv.slice(2)).catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`babel-native-auth: ${message}\n`);
    process.exitCode = 1;
  });
}
