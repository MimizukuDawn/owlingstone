// Owlingstone shared-games API (Cloudflare Worker + D1, Free plan).
//
//   POST   /v1/games          {"install": "<uuid>", "games": [summary, ...], "feedback": [{kind, text}, ...]}
//                              (up to 20 games and 5 feedback messages; either list may be empty)
//   DELETE /v1/installs/<id>  delete every game and feedback sent by this install id
//   GET    /v1/admin/feedback?after=<id>   (header "authorization: Bearer <ADMIN_KEY>") new feedback
//   POST   /v1/admin/feedback/done {"ids": [...]}   mark feedback as handled (same header)
//
// Built to stay on the Free plan: one request carries up to 20 games, bodies are capped, each install id
// may add MAX_PER_INSTALL games a day, each address may make RATE_LIMIT requests a minute (Cloudflare's
// rate-limit binding keeps the address in memory only; it is never stored), and above DAILY_GUARD games and
// feedback a day the API answers 503 so apps keep their games queued for tomorrow.
// One invalid game no longer sinks the batch: valid ones are stored and the answer lists the rejected ones. Stats are NOT served here (a daily GitHub Action builds
// stats.json and GitHub serves it), so reading stats costs no Worker request.
// No IP address, header or cookie is stored. Only fields kept by validateGame() reach the database.

import { gameQuality, validateFeedback, validateGame } from "./validate.js";

const MAX_BODY = 64 * 1024;          // bytes
const MAX_GAMES = 20;                // per request
const MAX_PER_INSTALL = 40;          // games per install id per day
const MAX_FEEDBACK = 5;              // feedback messages per request and per install id per day
// D1 Free = 100,000 rows written a day, and every index on a table counts as one more written row: a game
// costs about 5 rows (game + day index + 2 counter upserts with their keys). 12,000 items leave room.
const DAILY_GUARD = 12000;
const MIN_APP = [1, 8, 0];           // sharing started in 1.8.0; anything older is not a real app
const INSTALL = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const json = (status, body) => new Response(JSON.stringify(body), {
  status,
  headers: { "content-type": "application/json", "cache-control": "no-store", "x-content-type-options": "nosniff" },
});

const today = () => new Date().toISOString().slice(0, 10);

async function readBody(request) {
  const declared = Number(request.headers.get("content-length") || 0);
  if (declared > MAX_BODY) return null;
  const reader = request.body?.getReader();
  if (!reader) return null;
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > MAX_BODY) return null;
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let at = 0;
  for (const c of chunks) { bytes.set(c, at); at += c.byteLength; }
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return undefined;
  }
}

async function addGames(request, env) {
  if (!(request.headers.get("content-type") || "").startsWith("application/json")) {
    return json(415, { error: "json only" });
  }
  const body = await readBody(request);
  if (body === null) return json(413, { error: "too large" });
  if (!body || typeof body !== "object" || !INSTALL.test(body.install ?? "")
      || !Array.isArray(body.games) || body.games.length > MAX_GAMES) {
    return json(400, { error: "bad request" });
  }
  const notes = body.feedback ?? [];
  if (!Array.isArray(notes) || notes.length > MAX_FEEDBACK || body.games.length + notes.length < 1) {
    return json(400, { error: "bad request" });
  }
  const checked = body.games.map((g) => {
    const v = validateGame(g);
    return v && knownApp(v.app) ? v : null;
  });
  const checkedNotes = notes.map(validateFeedback);
  const rejected = { games: indexesOf(checked, null), feedback: indexesOf(checkedNotes, null) };
  const games = checked.filter(Boolean), feedback = checkedNotes.filter(Boolean);
  if (!games.length && !feedback.length) return json(422, { error: "invalid content", rejected });
  const build = games[0]?.app ?? feedback[0].app ?? "0.0.0";

  const day = today();
  const counts = await env.DB.batch([
    env.DB.prepare("SELECT n FROM day_count WHERE day = ?1").bind(day),
    env.DB.prepare("SELECT n FROM install_count WHERE install = ?1 AND day = ?2").bind(body.install, day),
    env.DB.prepare("SELECT COUNT(*) AS n FROM feedback WHERE install = ?1 AND day = ?2").bind(body.install, day),
  ]);
  const todayTotal = counts[0].results[0]?.n ?? 0, mine = counts[1].results[0]?.n ?? 0;
  const myNotes = counts[2].results[0]?.n ?? 0;
  if (todayTotal + games.length + feedback.length > DAILY_GUARD) {
    return json(503, { error: "daily limit, retry tomorrow" });
  }
  if (mine + games.length > MAX_PER_INSTALL || myNotes + feedback.length > MAX_FEEDBACK) {
    return json(429, { error: "too many today" });
  }

  const insert = env.DB.prepare(
    `INSERT INTO games (install, day, build, mode, hero, place, comp, quality, summary)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)`);
  const note = env.DB.prepare("INSERT INTO feedback (install, day, build, kind, text) VALUES (?1, ?2, ?3, ?4, ?5)");
  const k = games.length + feedback.length;
  await env.DB.batch([
    ...games.map((g) => insert.bind(body.install, day, g.app, g.mode, g.hero, g.place, g.comp, gameQuality(g),
      JSON.stringify(g))),
    ...feedback.map((f) => note.bind(body.install, day, f.app ?? build, f.kind, f.text)),
    env.DB.prepare("INSERT INTO day_count (day, n) VALUES (?1, ?2) ON CONFLICT (day) DO UPDATE SET n = n + ?2")
      .bind(day, k),
    env.DB.prepare(`INSERT INTO install_count (install, day, n) VALUES (?1, ?2, ?3)
      ON CONFLICT (install, day) DO UPDATE SET n = n + ?3`).bind(body.install, day, k),
  ]);
  return json(200, { stored: games.length, feedback: feedback.length, rejected });
}

const indexesOf = (list, value) => list.flatMap((x, i) => (x === value ? [i] : []));

function knownApp(version) {
  const v = version.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (v[i] !== MIN_APP[i]) return v[i] > MIN_APP[i];
  return true;
}

// Loose per-address limit against scripts that make new install ids. Shared addresses (a family, a
// campus) stay far below it: a normal app sends about one request per game.
async function overRateLimit(request, env) {
  if (!env.LIMITER) return false;                  // binding missing (local tests): no limit
  const ip = request.headers.get("cf-connecting-ip") || "unknown";
  const { success } = await env.LIMITER.limit({ key: ip });
  return !success;
}

async function deleteInstall(id, env) {
  if (!INSTALL.test(id)) return json(400, { error: "bad id" });
  const r = await env.DB.batch([
    env.DB.prepare("DELETE FROM games WHERE install = ?1").bind(id),
    env.DB.prepare("DELETE FROM install_count WHERE install = ?1").bind(id),
    env.DB.prepare("DELETE FROM feedback WHERE install = ?1").bind(id),
  ]);
  return json(200, { deleted: r[0].meta?.changes ?? 0 });
}

// Constant-time comparison of the admin key (the key itself is a Worker secret, never in the code).
async function isAdmin(request, env) {
  const given = (request.headers.get("authorization") || "").replace(/^Bearer /, "");
  if (!env.ADMIN_KEY || env.ADMIN_KEY.length < 32 || !given) return false;
  const enc = new TextEncoder();
  const [a, b] = await Promise.all([given, env.ADMIN_KEY].map((x) => crypto.subtle.digest("SHA-256", enc.encode(x))));
  return crypto.subtle.timingSafeEqual(a, b);
}

async function listFeedback(url, env) {
  const after = Math.max(0, Number.parseInt(url.searchParams.get("after") || "0", 10) || 0);
  const r = await env.DB.prepare(
    "SELECT id, day, build, kind, text FROM feedback WHERE done = 0 AND id > ?1 ORDER BY id LIMIT 500").bind(after).all();
  return json(200, { feedback: r.results });
}

async function markDone(request, env) {
  const body = await readBody(request);
  const ids = body?.ids;
  if (!Array.isArray(ids) || ids.length > 500 || !ids.every((i) => Number.isInteger(i) && i > 0)) {
    return json(400, { error: "bad ids" });
  }
  const done = env.DB.prepare("UPDATE feedback SET done = 1 WHERE id = ?1");
  if (ids.length) await env.DB.batch(ids.map((i) => done.bind(i)));
  return json(200, { done: ids.length });
}

export { knownApp };

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    try {
      const write = (url.pathname === "/v1/games" && request.method === "POST")
        || (url.pathname.startsWith("/v1/installs/") && request.method === "DELETE");
      if (write && await overRateLimit(request, env)) return json(429, { error: "slow down" });
      if (url.pathname === "/v1/games" && request.method === "POST") return await addGames(request, env);
      const m = url.pathname.match(/^\/v1\/installs\/([^/]+)$/);
      if (m && request.method === "DELETE") return await deleteInstall(m[1], env);
      if (url.pathname.startsWith("/v1/admin/")) {
        if (!(await isAdmin(request, env))) return json(401, { error: "unauthorized" });
        if (url.pathname === "/v1/admin/feedback" && request.method === "GET") return await listFeedback(url, env);
        if (url.pathname === "/v1/admin/feedback/done" && request.method === "POST") return await markDone(request, env);
        return json(404, { error: "not found" });
      }
      if (url.pathname === "/" && request.method === "GET") return json(200, { service: "owlingstone" });
      return json(404, { error: "not found" });
    } catch {
      return json(500, { error: "server error" });   // no details leaked
    }
  },
};
