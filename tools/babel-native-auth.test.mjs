import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  CookieJar,
  babelBackendCookieHeader,
  choosePhoneCodeFactor,
  loginWithPhoneCode,
  mintBabelSessionCookie,
  parseSetCookie,
  readNativeAuthStatus,
  startPhoneCodeLogin,
  verifyPhoneCodeLogin,
} from "./babel-native-auth.mjs";

test("parseSetCookie stores domain, path, and expiry metadata", () => {
  const cookie = parseSetCookie(
    "__client=client-value; Path=/; Domain=clerk.babel.audio; HttpOnly; Secure; SameSite=Lax",
    "https://clerk.babel.audio/v1/client",
  );

  assert.deepEqual(
    {
      name: cookie.name,
      value: cookie.value,
      domain: cookie.domain,
      path: cookie.path,
      httpOnly: cookie.httpOnly,
      secure: cookie.secure,
      sameSite: cookie.sameSite,
    },
    {
      name: "__client",
      value: "client-value",
      domain: "clerk.babel.audio",
      path: "/",
      httpOnly: true,
      secure: true,
      sameSite: "Lax",
    },
  );
});

test("CookieJar returns only cookies matching the request host", () => {
  const jar = new CookieJar([
    {
      name: "__client",
      value: "client-value",
      domain: "clerk.babel.audio",
      path: "/",
      expiresAt: null,
    },
    {
      name: "__session",
      value: "session-value",
      domain: "dashboard.babel.audio",
      path: "/",
      expiresAt: null,
    },
  ]);

  assert.equal(
    jar.headerForUrl("https://clerk.babel.audio/v1/client"),
    "__client=client-value",
  );
  assert.equal(
    jar.headerForUrl("https://dashboard.babel.audio/api/trpc/worker.get"),
    "__session=session-value",
  );
});

test("babelBackendCookieHeader emits Babel's required session and client_uat cookies", () => {
  const header = babelBackendCookieHeader("jwt-value", { now: () => 1782754800123 });

  assert.equal(
    header,
    "__session=jwt-value; __session_v1aVht_F=jwt-value; __client_uat=1782754800; __client_uat_v1aVht_F=1782754800",
  );
});

test("choosePhoneCodeFactor rejects sign-ins without a phone-code factor", () => {
  assert.throws(
    () =>
      choosePhoneCodeFactor({
        supported_first_factors: [{ strategy: "email_code", email_address_id: "idn_1" }],
      }),
    /phone_code is not an available first factor/,
  );
});

test("mintBabelSessionCookie mints a fresh Babel backend cookie from stored Clerk cookies", async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), "babel-native-auth-test-"));
  const storePath = path.join(temp, "store.json");
  await new CookieJar([
    {
      name: "__client",
      value: "client-cookie",
      domain: "clerk.babel.audio",
      path: "/",
      expiresAt: null,
    },
  ]).save(storePath);

  const calls = [];
  const result = await mintBabelSessionCookie({
    storePath,
    now: () => 1782754800000,
    fetchImpl: async (url, init) => {
      calls.push({ url: String(url), init });
      assert.match(init.headers.cookie, /__client=client-cookie/);
      if (String(url).includes("/v1/client/sessions/sess_native/tokens")) {
        return jsonResponse({ object: "token", jwt: jwtWithExpiry(1782754860) });
      }
      return jsonResponse({
        response: {
          object: "client",
          last_active_session_id: "sess_native",
          sessions: [{ id: "sess_native", status: "active" }],
        },
      });
    },
  });

  assert.equal(calls.length, 2);
  assert.equal(result.sessionId, "sess_native");
  assert.equal(result.expiresAt, "2026-06-29T17:41:00.000Z");
  assert.match(result.cookie, /__session=/);
  assert.match(result.cookie, /__session_v1aVht_F=/);
  assert.match(result.cookie, /__client_uat=1782754800/);
});

test("loginWithPhoneCode creates, verifies, stores Clerk cookies, and returns a Babel cookie", async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), "babel-native-auth-login-test-"));
  const storePath = path.join(temp, "store.json");
  const calls = [];

  const result = await loginWithPhoneCode({
    storePath,
    identifier: "+15555550123",
    code: "123456",
    now: () => 1782754800000,
    fetchImpl: async (url, init) => {
      calls.push({ url: String(url), body: init.body });
      if (String(url).includes("/v1/client/sign_ins/si_native/attempt_first_factor")) {
        assert.equal(init.body, "strategy=phone_code&code=123456");
        return jsonResponse({
          response: {
            object: "sign_in_attempt",
            id: "si_native",
            status: "complete",
            created_session_id: "sess_native",
          },
        });
      }
      if (String(url).includes("/v1/client/sign_ins/si_native/prepare_first_factor")) {
        assert.equal(init.body, "strategy=phone_code&phone_number_id=idn_phone");
        return jsonResponse({
          response: {
            object: "sign_in_attempt",
            id: "si_native",
            status: "needs_first_factor",
          },
        });
      }
      if (String(url).includes("/v1/client/sign_ins")) {
        assert.equal(init.body, "identifier=%2B15555550123");
        return jsonResponse(
          {
            response: {
              object: "sign_in_attempt",
              id: "si_native",
              status: "needs_first_factor",
              supported_first_factors: [
                { strategy: "phone_code", phone_number_id: "idn_phone" },
              ],
            },
          },
          ["__client=client-cookie; Domain=clerk.babel.audio; Path=/; HttpOnly; Secure"],
        );
      }
      if (String(url).includes("/v1/client/sessions/sess_native/tokens")) {
        return jsonResponse({ object: "token", jwt: jwtWithExpiry(1782754860) });
      }
      throw new Error(`unexpected URL ${url}`);
    },
  });

  const stored = JSON.parse(await fs.readFile(storePath, "utf8"));
  assert.equal(calls.length, 4);
  assert.equal(result.sessionId, "sess_native");
  assert.match(result.cookie, /__session=/);
  assert.equal(stored.cookies[0].name, "__client");
});

test("startPhoneCodeLogin stores pending sign-in state after sending the code", async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), "babel-native-auth-start-test-"));
  const storePath = path.join(temp, "store.json");
  const calls = [];

  const result = await startPhoneCodeLogin({
    storePath,
    identifier: "+15555550123",
    reset: true,
    fetchImpl: async (url, init) => {
      calls.push({ url: String(url), body: init.body });
      if (String(url).includes("/v1/client/sign_ins/si_native/prepare_first_factor")) {
        return jsonResponse({
          response: {
            object: "sign_in_attempt",
            id: "si_native",
            status: "needs_first_factor",
          },
        });
      }
      return jsonResponse(
        {
          response: {
            object: "sign_in_attempt",
            id: "si_native",
            status: "needs_first_factor",
            supported_first_factors: [
              { strategy: "phone_code", phone_number_id: "idn_phone" },
            ],
          },
        },
        ["__client=client-cookie; Domain=clerk.babel.audio; Path=/; HttpOnly; Secure"],
      );
    },
  });

  const stored = JSON.parse(await fs.readFile(storePath, "utf8"));
  assert.equal(calls.length, 2);
  assert.equal(result.pending, true);
  assert.equal(result.signInId, "si_native");
  assert.equal(stored.pendingSignIn.id, "si_native");
  assert.equal(stored.pendingSignIn.phoneNumberId, "idn_phone");
  assert.equal(stored.cookies[0].name, "__client");
});

test("verifyPhoneCodeLogin uses pending sign-in state and clears it after minting a session", async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), "babel-native-auth-verify-test-"));
  const storePath = path.join(temp, "store.json");
  const jar = new CookieJar([
    {
      name: "__client",
      value: "client-cookie",
      domain: "clerk.babel.audio",
      path: "/",
      expiresAt: null,
    },
  ]);
  jar.pendingSignIn = {
    id: "si_native",
    phoneNumberId: "idn_phone",
    createdAt: "2026-06-29T17:40:00.000Z",
  };
  await jar.save(storePath);
  const calls = [];

  const result = await verifyPhoneCodeLogin({
    storePath,
    code: "123456",
    now: () => 1782754800000,
    fetchImpl: async (url, init) => {
      calls.push({ url: String(url), body: init.body });
      assert.match(init.headers.cookie, /__client=client-cookie/);
      if (String(url).includes("/v1/client/sign_ins/si_native/attempt_first_factor")) {
        assert.equal(init.body, "strategy=phone_code&code=123456");
        return jsonResponse({
          response: {
            object: "sign_in_attempt",
            id: "si_native",
            status: "complete",
            created_session_id: "sess_native",
          },
        });
      }
      if (String(url).includes("/v1/client/sessions/sess_native/tokens")) {
        return jsonResponse({ object: "token", jwt: jwtWithExpiry(1782754860) });
      }
      throw new Error(`unexpected URL ${url}`);
    },
  });

  const stored = JSON.parse(await fs.readFile(storePath, "utf8"));
  assert.equal(calls.length, 2);
  assert.equal(result.sessionId, "sess_native");
  assert.equal(stored.pendingSignIn, undefined);
  assert.match(result.cookie, /__session=/);
});

test("readNativeAuthStatus reports pending phone-code login state", async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), "babel-native-auth-status-test-"));
  const storePath = path.join(temp, "store.json");
  const jar = new CookieJar([
    {
      name: "__client",
      value: "client-cookie",
      domain: "clerk.babel.audio",
      path: "/",
      expiresAt: null,
    },
  ]);
  jar.pendingSignIn = {
    id: "si_native",
    phoneNumberId: "idn_phone",
    createdAt: "2026-06-29T17:40:00.000Z",
  };
  await jar.save(storePath);

  const status = await readNativeAuthStatus({
    storePath,
    fetchImpl: async () =>
      jsonResponse({
        response: {
          object: "client",
          sessions: [],
        },
      }),
  });

  assert.equal(status.configured, true);
  assert.equal(status.active, false);
  assert.equal(status.pendingLogin, true);
  assert.equal(status.pendingSignInId, "si_native");
});

test("mintBabelSessionCookie tells the user to verify a pending login first", async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), "babel-native-auth-cookie-pending-test-"));
  const storePath = path.join(temp, "store.json");
  const jar = new CookieJar([
    {
      name: "__client",
      value: "client-cookie",
      domain: "clerk.babel.audio",
      path: "/",
      expiresAt: null,
    },
  ]);
  jar.pendingSignIn = {
    id: "si_native",
    phoneNumberId: "idn_phone",
    createdAt: "2026-06-29T17:40:00.000Z",
  };
  await jar.save(storePath);

  await assert.rejects(
    () =>
      mintBabelSessionCookie({
        storePath,
        fetchImpl: async () =>
          jsonResponse({
            response: {
              object: "client",
              sessions: [],
            },
          }),
      }),
    /verify-login --code/,
  );
});

test("loginWithPhoneCode rejects empty verification codes before attempting auth", async () => {
  await assert.rejects(
    () =>
      loginWithPhoneCode({
        identifier: "+15555550123",
        code: "",
        promptImpl: async () => "",
        fetchImpl: async (url, init) => {
          if (String(url).includes("/v1/client/sign_ins/si_native/prepare_first_factor")) {
            return jsonResponse({
              response: {
                object: "sign_in_attempt",
                id: "si_native",
                status: "needs_first_factor",
              },
            });
          }
          return jsonResponse({
            response: {
              object: "sign_in_attempt",
              id: "si_native",
              status: "needs_first_factor",
              supported_first_factors: [
                { strategy: "phone_code", phone_number_id: "idn_phone" },
              ],
            },
          });
        },
      }),
    /phone verification code is empty/,
  );
});

function jsonResponse(payload, setCookies = []) {
  return {
    ok: true,
    status: 200,
    headers: {
      getSetCookie() {
        return setCookies;
      },
    },
    async text() {
      return JSON.stringify(payload);
    },
  };
}

function jwtWithExpiry(exp) {
  const header = Buffer.from(JSON.stringify({ alg: "none" })).toString("base64url");
  const payload = Buffer.from(JSON.stringify({ exp })).toString("base64url");
  return `${header}.${payload}.signature`;
}
