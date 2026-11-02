// Strict check of one game summary sent by the app. Anything unexpected is refused, never stored.
// Only the fields listed here are kept, so the app cannot send (by mistake or not) anything else.

const ID = /^[A-Za-z0-9_]{1,40}$/;              // card ids like BG25_008 or BG25_008_G
const NAME = /^[\p{L}\p{N} '’.,:!&()-]{1,60}$/u; // hero, comp or deity names
const RACES = new Set(["ABERRATION", "BEAST", "DEMON", "DRAGON", "ELEMENTAL", "MECHANICAL", "MURLOC",
  "NAGA", "PIRATE", "QUILBOAR", "UNDEAD"]);

const int = (v, lo, hi) => Number.isInteger(v) && v >= lo && v <= hi;

function minion(m) {
  if (!m || typeof m !== "object" || !ID.test(m.id) || !int(m.a, 0, 100000) || !int(m.h, 0, 100000)) {
    return null;
  }
  return { id: m.id, a: m.a, h: m.h, g: m.g === true };
}

function board(list, max = 7) {
  if (!Array.isArray(list) || list.length > max) return null;
  const out = list.map(minion);
  return out.includes(null) ? null : out;
}

function fight(f) {
  if (!f || typeof f !== "object" || !int(f.t, 1, 60) || !int(f.tier, 1, 7) || !int(f.otier, 1, 7)
      || !int(f.hp, -100, 200) || !["W", "L", "T"].includes(f.r) || !int(f.d, 0, 100)) {
    return null;
  }
  const me = board(f.me), opp = board(f.opp);
  if (!me || !opp) return null;
  return { t: f.t, tier: f.tier, otier: f.otier, hp: f.hp, r: f.r, d: f.d, me, opp };
}

export function validateGame(g) {
  if (!g || typeof g !== "object" || g.v !== 1) return null;
  if (typeof g.app !== "string" || !/^\d+\.\d+\.\d+$/.test(g.app)) return null;
  if (!["solo", "duos"].includes(g.mode) || !NAME.test(g.hero ?? "") || !int(g.place, 1, 8)) return null;
  if (g.deity != null && !NAME.test(g.deity)) return null;
  if (g.comp != null && !/^[a-z0-9-]{1,40}$/.test(g.comp)) return null;
  if (!Array.isArray(g.races) || g.races.length > 6 || !g.races.every((r) => RACES.has(r))) return null;
  if (g.mode === "solo" && g.place > 8) return null;
  if (g.mode === "duos" && g.place > 4) return null;
  const final = board(g.board);
  if (!final || !Array.isArray(g.fights) || g.fights.length > 40) return null;
  const fights = g.fights.map(fight);
  if (fights.includes(null)) return null;
  if (!int(g.turns, 1, 60) || !Array.isArray(g.actions) || g.actions.length !== g.turns
      || !g.actions.every((a) => int(a, 0, 999)) || !int(g.end_hp, -100, 200) || typeof g.conceded !== "boolean") {
    return null;
  }
  return { v: 1, app: g.app, mode: g.mode, hero: g.hero, deity: g.deity ?? null, comp: g.comp ?? null,
    races: [...new Set(g.races)].sort(), place: g.place, board: final, fights,
    turns: g.turns, actions: g.actions, end_hp: g.end_hp, conceded: g.conceded };
}

// Is this game normal play? Games that are not stay stored but are left out of the shared stats.
//   "afk"           3 or more turns in a row with no action at all (not one buy, sell, play, roll, level,
//                   freeze or hero power). Turn 1 (hero choice) and the last turn (cut by the end) don't count.
//                   One or two empty turns are normal: heroes that save gold or wait a turn on purpose,
//                   or a player who only uses the hero power (that counts as an action).
//   "early_concede" the player conceded (or left) while still healthy: 15 health or more, or before turn 6.
//                   Conceding at low health when the next fight is lost anyway is normal play.
// Rules live here (server side) so they can be tuned without an app update.
export const AFK_STREAK = 3;
export const CONCEDE_MIN_HP = 15;
export const CONCEDE_EARLY_TURN = 6;

export function gameQuality(g) {
  let streak = 0, worst = 0;
  for (let t = 2; t < g.turns; t++) {              // turns 2 .. last-1 (actions[t-1] = turn t)
    streak = g.actions[t - 1] === 0 ? streak + 1 : 0;
    worst = Math.max(worst, streak);
  }
  if (worst >= AFK_STREAK) return "afk";
  const left = g.conceded || (g.place > 1 && g.end_hp > 0);
  if (left && (g.end_hp >= CONCEDE_MIN_HP || g.turns < CONCEDE_EARLY_TURN)) return "early_concede";
  return "ok";
}

// Feedback typed by a player: a kind and up to 1000 characters of plain text (control characters removed).
export function validateFeedback(f) {
  if (!f || typeof f !== "object" || !["idea", "bug", "other"].includes(f.kind) || typeof f.text !== "string") {
    return null;
  }
  if (f.app != null && (typeof f.app !== "string" || !/^\d+\.\d+\.\d+$/.test(f.app))) return null;
  const text = f.text.replace(/[\u0000-\u0008\u000B-\u001F\u007F]/g, "").trim();
  if (text.length < 3 || text.length > 1000) return null;
  return { kind: f.kind, text, app: f.app ?? null };
}
