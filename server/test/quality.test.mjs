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
