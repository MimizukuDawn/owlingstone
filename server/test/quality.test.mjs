// node --test server/test  : rules that keep abandoned or AFK games out of the shared stats.
import assert from "node:assert/strict";
import { test } from "node:test";
import { gameQuality, validateGame } from "../src/validate.js";

const game = (actions, extra = {}) => ({ turns: actions.length, actions, place: 5, end_hp: -2, conceded: false, ...extra });

test("normal game is ok", () => assert.equal(gameQuality(game([2, 1, 6, 8, 5, 2, 30, 24, 21, 29])), "ok"));
test("one or two empty turns (saving gold, waiting a turn) stay ok", () => {
  assert.equal(gameQuality(game([2, 0, 6, 0, 0, 3, 9, 9])), "ok");
});
test("hero-power-only turns count as play", () => assert.equal(gameQuality(game([2, 1, 1, 1, 1, 1, 1])), "ok"));
test("three empty turns in a row is AFK", () => assert.equal(gameQuality(game([2, 3, 0, 0, 0, 0, 0])), "afk"));
test("empty last turn (cut by game end) is ignored", () => assert.equal(gameQuality(game([2, 3, 4, 0, 0, 0])), "ok"));
test("concede while healthy is early_concede", () => {
  assert.equal(gameQuality(game([2, 3, 4, 5, 6, 7, 8], { conceded: true, end_hp: 24, place: 8 })), "early_concede");
});
test("concede on turn 4 is early_concede even at low health", () => {
  assert.equal(gameQuality(game([2, 3, 4, 5], { conceded: true, end_hp: 5, place: 8 })), "early_concede");
});
test("concede at low health late is normal", () => {
  assert.equal(gameQuality(game([2, 3, 4, 5, 6, 7, 8, 9, 9, 9], { conceded: true, end_hp: 4, place: 6 })), "ok");
});
test("leaving without the CONCEDED tag is detected by health above 0", () => {
  assert.equal(gameQuality(game([2, 3, 4, 5, 6, 7, 8], { end_hp: 30, place: 8 })), "early_concede");
});
test("winner keeps health and is ok", () => assert.equal(gameQuality(game([2, 3, 4, 5, 6, 7, 8], { place: 1, end_hp: 12 })), "ok"));

test("validator needs one action count per turn", () => {
  const g = { v: 1, app: "1.8.0", mode: "solo", hero: "Rafaam", races: [], place: 3, board: [], fights: [],
    turns: 3, actions: [1, 2, 3], end_hp: 0, conceded: false };
  assert.ok(validateGame(g));
  assert.equal(validateGame({ ...g, actions: [1, 2] }), null);
  assert.equal(validateGame({ ...g, conceded: "no" }), null);
});

import { knownApp } from "../src/worker.js";
test("only apps that can share are accepted", () => {
  assert.equal(knownApp("1.8.0"), true);
  assert.equal(knownApp("1.9.0"), true);
  assert.equal(knownApp("2.0.0"), true);
  assert.equal(knownApp("1.7.9"), false);
  assert.equal(knownApp("0.9.0"), false);
});

import worker from "../src/worker.js";

function fakeDb() {
  const writes = [];
  const stmt = (sql) => ({ sql, bind: (...args) => ({ sql, args }) });
  return {
    writes,
    prepare: stmt,
    async batch(list) {
      if (list[0].sql.startsWith("SELECT")) return list.map(() => ({ results: [] }));
      writes.push(...list);
      return list.map(() => ({ meta: { changes: 1 } }));
    },
  };
}

const goodGame = { v: 1, app: "1.9.0", mode: "solo", hero: "Rafaam", races: [], place: 3, board: [], fights: [],
  turns: 2, actions: [1, 2], end_hp: 0, conceded: false };
const post = (body) => new Request("https://x/v1/games", {
  method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
const install = "0b7c2b8e-6c1f-4c5e-9a3d-1f2e3d4c5b6a";

test("one invalid game does not sink the batch", async () => {
  const env = { DB: fakeDb() };
  const r = await worker.fetch(post({ install, games: [goodGame, { ...goodGame, place: 99 }, goodGame] }), env);
  assert.equal(r.status, 200);
  const body = await r.json();
  assert.equal(body.stored, 2);
  assert.deepEqual(body.rejected.games, [1]);
  assert.equal(env.DB.writes.filter((w) => w.sql.includes("INSERT INTO games")).length, 2);
});

test("all invalid is 422, and the address limit answers 429", async () => {
  assert.equal((await worker.fetch(post({ install, games: [{ ...goodGame, app: "1.7.1" }] }), { DB: fakeDb() })).status, 422);
  const env = { DB: fakeDb(), LIMITER: { limit: async () => ({ success: false }) } };
  assert.equal((await worker.fetch(post({ install, games: [goodGame] }), env)).status, 429);
});
