//! What the window shows, built in the log reader thread.
//!
//! - `make_snapshot(game, ...)` turns the game into a `Snapshot`: plain data plus all the advice
//!   (heroes, comp played, buys, turn plan, Trinkets, fight odds, tier list of comps). The window only
//!   draws it, so a slow simulation never freezes it.
//! - `Reader` runs in its own thread: it follows Power.log (`watcher.rs`), sends a new snapshot when
//!   something changed, and at the end of a game learns the tribes of neutrals, writes the game to the
//!   history and queues it for sharing (if the player said yes). A panic in there (unexpected line,
//!   simulator bug) never stops it: it is written to error.log and, after a few in a row, the reader
//!   starts over from a fresh parser.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::advice::{self, ShopPick};
use crate::cards::{self, Card};
use crate::choices::{self, ChoiceAdvice};
use crate::combat::MinionSnap;
use crate::comps;
use crate::game::{Bonuses, Entity, Game};
use crate::history;
use crate::knowledge::{self, HeroStat, Knowledge};
use crate::planner::{self, Caches, CombatInfo, Plan, TurnAdvice};
use crate::pool;
use crate::powerlog::PowerLogParser;
use crate::rules::{self, Ctx, Verdict};
use crate::share::Share;
use crate::trinkets::{self, OfferHint, TrinketPick};
use crate::watcher::{LogWatcher, Status};

/// Lobby comps listed in the header at the start of a game (best first).
pub const LOBBY_SHOWN: usize = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct FightSummary {
    pub turn: i64,
    pub opponent: Option<i64>,
    pub result: char,
    pub damage: i64,
}

/// My last fight against a lobby player.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerFight {
    pub turn: i64,
    pub result: char,
    pub damage: i64,
    /// Their board (base card ids, for images).
    pub board: Vec<MinionSnap>,
}

/// One of the 8 players, for the Lobby panel.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLine {
    pub id: Option<i64>,
    pub hero: String,
    pub hero_id: String,
    pub place: Option<i64>,
    pub health: i64,
    pub tier: Option<i64>,
    pub triples: i64,
    pub dead: bool,
    pub me: bool,
    pub next: bool,
    pub fight: Option<PlayerFight>,
}

/// A hero offered at the start.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroOption {
    pub name: String,
    pub card_id: String,
    pub stat: Option<HeroStat>,
    /// (comp id, the hero's average place with it, games): comps where the hero is among the best.
    pub best_comps: Vec<(String, f64, f64)>,
    /// The hero's own curve / power note (data/hero_curves.yaml).
    pub note: String,
}

/// The comp my board plays (owned copy of `comps::CompMatch`).
#[derive(Debug, Clone, PartialEq)]
pub struct Playing {
    pub comp_id: String,
    pub name: String,
    pub confidence: i64,
    pub have: Vec<Arc<Card>>,
    pub missing_core: Vec<Arc<Card>>,
    pub missing_addons: Vec<Arc<Card>>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanAdvice {
    pub advice: TurnAdvice,
    pub fight: Arc<Plan>,
    /// (comp id, avg, games): best comps of my hero in this lobby.
    pub hero_best: Vec<(String, f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrinketAdvice {
    pub offer: Vec<TrinketPick>,
    pub hint: Option<OfferHint>,
}

/// A comp of the tier list (Comps tab), best first.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedComp {
    pub comp_id: String,
    /// "S".."D" or "?".
    pub letter: &'static str,
    pub avg: f64,
    pub verdict: Verdict,
    pub playing: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Advice {
    pub heroes: Vec<HeroOption>,
    pub playing: Option<Playing>,
    pub alt: Option<Playing>,
    /// Comps of the lobby (when nothing is played yet), best first.
    pub lobby: Vec<String>,
    pub buy: Vec<ShopPick>,
    pub plan: Option<PlanAdvice>,
    pub trinkets: Option<TrinketAdvice>,
    /// Cards of the comp played (★ in the tier list).
    pub comp_cards: HashSet<String>,
    /// Cards to buy (▶ in the tier list).
    pub buy_cards: HashSet<String>,
    /// Tier list of the comps of this lobby (my rules applied).
    pub ranked: Vec<RankedComp>,
    /// Comps hidden by my rules.
    pub hidden: Vec<String>,
}

/// Everything the window shows. Built by the reader thread, never changed after.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub in_game: bool,
    pub log_status: Option<Status>,
    pub logs_dir: Option<PathBuf>,
    pub hero: Option<String>,
    pub hero_id: Option<String>,
    pub tier: Option<i64>,
    pub turn: i64,
    pub health: Option<i64>,
    pub gold: Option<i64>,
    /// Tribes seen in the shop (sorted), None before any.
    pub races: Option<Vec<String>>,
    pub races_complete: bool,
    pub possible_races: Vec<String>,
    pub deity: Option<String>,
    pub duos: bool,
    pub hero_options: Vec<String>,
    pub hero_option_ids: Vec<String>,
    /// Base card ids (golden -> normal).
    pub board: Vec<String>,
    pub hand: Vec<String>,
    pub shop: Vec<String>,
    /// Golden cards among board + hand: they don't count for a triple.
    pub golden: Vec<String>,
    pub over: bool,
    pub placement: Option<i64>,
    pub players: Vec<PlayerLine>,
    pub bonuses: Option<Bonuses>,
    /// (name, card id) of my hero powers.
    pub hero_powers: Vec<(String, String)>,
    pub fights: Vec<FightSummary>,
    pub trinket_offer: Vec<(String, i64)>,
    pub trinkets: Vec<String>,
    pub combat: Option<CombatInfo>,
    pub advice: Advice,
    /// The choice on screen (hero pick, Trinket offer, Discover) with the chance of each option.
    pub choice: Option<ChoiceAdvice>,
    /// Problems in the data files (shown in red; the rest still works).
    pub data_errors: Vec<String>,
}

impl Snapshot {
    /// Tribes for the rules and the pool: only once all are known.
    pub fn sure_races(&self) -> Option<&[String]> {
        if self.races_complete { self.races.as_deref() } else { None }
    }

    /// What the header says out of a game: waiting, or what is missing to read the game.
    pub fn waiting_text(&self) -> String {
        match self.log_status {
            Some(Status::NoFolder) => {
                "Hearthstone not found. Start the game, or right-click the tray icon > Choose Hearthstone folder…".into()
            }
            Some(Status::NoLog) => format!(
                "No Power.log yet in {}. If the game is open, restart it once (logging is turned on at its start).",
                self.logs_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "the Logs folder".into())
            ),
            _ => "Waiting for a Battlegrounds game…".into(),
        }
    }
}

/// Things kept between two snapshots of the same game (simulations, pool of the lobby).
#[derive(Default)]
pub struct State {
    pub caches: Caches,
    pool: Option<(PoolKey, Arc<Vec<Arc<Card>>>)>,
}

/// (tier, tribes, duos, card list generation).
type PoolKey = (i64, Vec<String>, bool, u64);

impl State {
    fn pool_up_to(&mut self, tier: i64, races: Option<&[String]>, duos: bool) -> Arc<Vec<Arc<Card>>> {
        let key = (tier, races.map(<[String]>::to_vec).unwrap_or_default(), duos, cards::generation());
        if let Some((k, cards)) = &self.pool {
            if *k == key {
                return cards.clone();
            }
        }
        let cards = Arc::new(pool::pool_up_to(&cards::db(), tier, races, &pool::load_subsets(), duos));
        self.pool = Some((key, cards.clone()));
        cards
    }
}

fn base_ids(entities: &[&Entity]) -> Vec<String> {
    let db = cards::db();
    entities.iter().filter(|e| !e.card_id.is_empty()).filter_map(|e| db.base(&e.card_id).map(|c| c.id.clone())).collect()
}

fn cards_of(ids: &[String]) -> Vec<Arc<Card>> {
    let db = cards::db();
    ids.iter().filter_map(|i| db.get(i).cloned()).collect()
}

/// The 8 players for the Lobby panel: hero, place, health, tier, triples, last fight against me.
pub fn lobby_players(game: &Game) -> Vec<PlayerLine> {
    let db = cards::db();
    let (me, next) = (game.my_player_id(), game.next_opponent_id());
    game.lobby_heroes()
        .into_iter()
        .map(|h| {
            let pid = h.opt_int("PLAYER_ID");
            let fight = pid.filter(|p| Some(*p) != me).and_then(|p| game.last_fight_against(p));
            let health = h.health_left();
            PlayerLine {
                id: pid,
                hero: h.name(),
                hero_id: h.card_id.clone(),
                place: h.opt_int("PLAYER_LEADERBOARD_PLACE"),
                health,
                tier: h.opt_int("PLAYER_TECH_LEVEL"),
                triples: h.int_or("PLAYER_TRIPLES", 0),
                dead: health <= 0,
                me: pid.is_some() && pid == me,
                next: pid.is_some() && pid == next && pid != me,
                fight: fight.map(|f| PlayerFight {
                    turn: f.turn,
                    result: f.result,
                    damage: f.damage,
                    board: f
                        .board
                        .iter()
                        .flatten()
                        .map(|m| MinionSnap { card_id: db.base_id(&m.card_id).to_string(), ..m.clone() })
                        .collect(),
                }),
            }
        })
        .collect()
}

fn playing(m: &comps::CompMatch) -> Playing {
    Playing {
        comp_id: m.comp.id.clone(),
        name: m.comp.name.clone(),
        confidence: m.confidence,
        have: m.have.clone(),
        missing_core: m.missing_core.clone(),
        missing_addons: m.missing_addons.clone(),
        notes: m.notes.clone(),
    }
}

/// Copy of what the window shows, with all the advice. Runs in the reader thread.
pub fn make_snapshot(game: Option<&mut Game>, state: &mut State) -> Snapshot {
    let k = knowledge::get();
    let mut snap = Snapshot { data_errors: k.errors.clone(), ..Default::default() };
    let Some(game) = game.filter(|g| g.is_battlegrounds()) else { return snap };
    if game.is_over() {
        // Back in the menu: the waiting view with my place, no odds or plan of a finished game.
        let hero = game.my_hero();
        return Snapshot {
            over: true,
            placement: game.my_placement(),
            hero: hero.map(Entity::name),
            hero_id: hero.map(|h| h.card_id.clone()),
            turn: game.turn(),
            duos: game.is_duos(),
            players: lobby_players(game),
            ..snap
        };
    }
    let combat = planner::combat_info(game, &mut state.caches);
    let plan = planner::fight_plan(&k, game, &mut state.caches);
    let game: &Game = game;
    let mine: Vec<&Entity> = game.my_board().into_iter().chain(game.my_hand()).collect();
    let mut races = game.available_races();
    races.sort();
    let hero = game.my_hero();
    snap = Snapshot {
        in_game: true,
        hero: hero.map(Entity::name),
        hero_id: hero.map(|h| h.card_id.clone()),
        tier: game.tavern_tier(),
        turn: game.turn(),
        health: game.my_health(),
        gold: game.my_gold(),
        races: if game.has_shop_races() { Some(races) } else { None },
        races_complete: game.races_complete(),
        possible_races: game.possible_races(),
        deity: game.deity(),
        duos: game.is_duos(),
        hero_options: game.hero_options().iter().map(|e| e.name()).collect(),
        hero_option_ids: game.hero_options().iter().map(|e| e.card_id.clone()).collect(),
        board: base_ids(&game.my_board()),
        hand: base_ids(&game.my_hand()),
        shop: base_ids(&game.shop()),
        golden: base_ids(&mine.iter().copied().filter(|e| e.is_golden()).collect::<Vec<_>>()),
        over: game.is_over(),
        placement: game.my_placement(),
        players: lobby_players(game),
        bonuses: Some(game.my_bonuses()),
        hero_powers: game.my_hero_powers().iter().map(|e| (e.name(), e.card_id.clone())).collect(),
        fights: game
            .fights
            .iter()
            .map(|f| FightSummary { turn: f.turn, opponent: f.opponent, result: f.result, damage: f.damage })
            .collect(),
        trinket_offer: game.trinket_offer().iter().map(|(e, cost)| (e.card_id.clone(), *cost)).collect(),
        trinkets: game.my_trinkets().iter().map(|e| e.card_id.clone()).collect(),
        combat,
        data_errors: snap.data_errors,
        ..Default::default()
    };
    snap.advice = make_advice(&k, &snap, plan, state);
    snap.choice = choice_advice(&k, game, &snap, state);
    snap
}

/// The choice on screen: heroes first (start of the game), then a pick the log names (Discover,
/// Trinkets...), then a Trinket offer seen without its choice lines.
fn choice_advice(k: &Knowledge, game: &Game, s: &Snapshot, state: &mut State) -> Option<ChoiceAdvice> {
    if !s.advice.heroes.is_empty() {
        return choices::for_heroes(&s.advice.heroes, &s.hero_option_ids);
    }
    let offer = s.advice.trinkets.as_ref().filter(|t| !t.offer.is_empty());
    if let Some((c, options)) = game.open_choice() {
        let db = cards::db();
        let ids: Vec<String> = options.iter().map(|e| e.card_id.clone()).collect();
        if options.iter().all(|e| e.is("CARDTYPE", "BATTLEGROUND_TRINKET")) {
            if let Some(t) = offer {
                return choices::for_trinkets(&t.offer, &ids);
            }
        }
        let cards: Vec<Arc<Card>> = ids.iter().filter_map(|id| db.base(id).or_else(|| db.get(id)).cloned()).collect();
        if cards.len() == options.len() {
            let board = cards_of(&s.board);
            let hand = cards_of(&s.hand);
            let ctx = ctx_of(s);
            let targets = advice::target_comps(k, &board, &hand, &ctx);
            let owned = advice::non_golden(&board.iter().chain(&hand).cloned().collect::<Vec<_>>(), &s.golden);
            let fights = if options.iter().any(|e| e.is_minion()) { planner::option_fights(game, &options, &mut state.caches) } else { None };
            let source = c.source.and_then(|id| game.entity(id)).map(Entity::name);
            return choices::for_discover(
                k,
                source.as_deref(),
                &cards,
                &board,
                &hand,
                &owned,
                &targets,
                fights.as_deref(),
                s.turn,
                s.tier.unwrap_or(1),
            );
        }
    }
    let t = offer?;
    let screen: Vec<String> = s.trinket_offer.iter().map(|(id, _)| id.clone()).collect();
    choices::for_trinkets(&t.offer, &screen)
}

fn ctx_of(snap: &Snapshot) -> Ctx {
    Ctx {
        races: snap.sure_races().map(<[String]>::to_vec),
        deity: snap.deity.clone(),
        hero: snap.hero.clone(),
        hero_power: if snap.hero_powers.is_empty() {
            None
        } else {
            Some(snap.hero_powers.iter().map(|(n, _)| n.clone()).collect())
        },
    }
}

fn make_advice(k: &Knowledge, s: &Snapshot, plan: Option<Arc<Plan>>, state: &mut State) -> Advice {
    let mut adv = Advice::default();
    let races = s.sure_races();
    let ctx = ctx_of(s);
    if !s.hero_options.is_empty() {
        let ids: HashMap<&String, &String> = s.hero_options.iter().zip(&s.hero_option_ids).collect();
        adv.heroes = advice::rank_heroes(&s.hero_options, &k.heroes)
            .into_iter()
            .map(|(name, stat)| HeroOption {
                card_id: ids.get(&name).map(|i| i.to_string()).unwrap_or_default(),
                stat,
                // All comps: the lobby's tribes are not in the log yet when heroes are offered.
                best_comps: comps::hero_best_comps(k, &name, races, 2).into_iter().map(|(c, a, g)| (c.id.clone(), a, g)).collect(),
                note: k.hero_curves.get(&name).map(|c| c.note.clone()).unwrap_or_default(),
                name,
            })
            .collect();
        tier_list(k, s, &ctx, &mut adv);
        return adv;
    }
    let board = cards_of(&s.board);
    let hand = cards_of(&s.hand);
    let shop = cards_of(&s.shop);
    let targets = advice::target_comps(k, &board, &hand, &ctx);

    // Trinkets: the offer ranked, or before it (turns 5 and 8) what my board unlocks.
    if !s.over {
        let db = cards::db();
        let offer: Vec<(Arc<Card>, i64)> =
            s.trinket_offer.iter().filter_map(|(id, cost)| db.get(id).map(|c| (c.clone(), *cost))).collect();
        adv.trinkets = if !offer.is_empty() {
            let powers: Vec<Arc<Card>> = s.hero_powers.iter().filter_map(|(_, id)| db.get(id).cloned()).collect();
            Some(TrinketAdvice {
                offer: trinkets::rank_offer(k, &offer, &board, &hand, &shop, &targets, &powers, s.gold),
                hint: None,
            })
        } else {
            trinkets::next_offer_hint(k, s.turn, &board, &hand, races, &targets).map(|h| TrinketAdvice { offer: vec![], hint: Some(h) })
        };
    }

    // Buys: the best 3 of the shop (none out of the recruit phase).
    if !shop.is_empty() && !s.over {
        let owned = advice::non_golden(&board.iter().chain(&hand).cloned().collect::<Vec<_>>(), &s.golden);
        let mut picks = advice::shop_advice(&shop, &owned, &targets);
        if let Some(plan) = plan {
            // Plan of the turn (planner.rs): comp points + what each card changes in the next fight.
            let have: HashSet<&str> = owned.iter().map(|c| c.id.as_str()).collect();
            let mut copies: Vec<(&str, usize)> = Vec::new();
            for c in &owned {
                match copies.iter_mut().find(|(i, _)| *i == c.id) {
                    Some(slot) => slot.1 += 1,
                    None => copies.push((&c.id, 1)),
                }
            }
            let count = |id: &str| copies.iter().find(|(i, _)| *i == id).map(|(_, n)| *n).unwrap_or(0);
            let main: Vec<_> = targets.iter().filter(|(_, w)| *w >= 0.5).map(|(c, _)| *c).collect();
            let mut wanted: Vec<Arc<Card>> =
                main.iter().flat_map(|c| &c.core).filter(|c| !have.contains(c.id.as_str())).cloned().collect();
            wanted.extend(owned.iter().filter(|c| count(&c.id) == 2).cloned());
            let mut protect: HashSet<String> =
                main.iter().flat_map(|c| c.core.iter().chain(&c.addons)).map(|c| c.id.clone()).collect();
            protect.extend(copies.iter().filter(|(_, n)| *n >= 2).map(|(i, _)| i.to_string()));
            let pool = state.pool_up_to(s.tier.unwrap_or(1).max(1), races, s.duos);
            let advice = planner::turn_plan(k, &plan, &picks, &wanted, &pool, &protect);
            picks = advice.buy.iter().map(|b| ShopPick { card: b.card.clone(), value: b.value, reasons: b.reasons.clone() }).collect();
            let hero_best = s
                .hero
                .as_deref()
                .map(|h| comps::hero_best_comps(k, h, races, 2).into_iter().map(|(c, a, g)| (c.id.clone(), a, g)).collect())
                .unwrap_or_default();
            adv.plan = Some(PlanAdvice { advice, fight: plan, hero_best });
        }
        picks.truncate(3);
        adv.buy_cards = picks.iter().map(|p| p.card.id.clone()).collect();
        adv.buy = picks;
    }

    let matches = comps::match_comps(&k.comps, &board, &hand, &k.rules, &ctx, 2, advice::PLAYING_MIN_CONFIDENCE);
    if let Some(best) = matches.first() {
        adv.comp_cards = best.comp.core.iter().chain(&best.comp.addons).map(|c| c.id.clone()).collect();
        adv.playing = Some(playing(best));
        adv.alt = matches.get(1).map(playing);
    } else {
        adv.lobby = comps::lobby_comps(k, &ctx).iter().map(|c| c.id.clone()).collect();
    }
    tier_list(k, s, &ctx, &mut adv);
    adv
}

/// Tier list (S to D): average place of the comp (my hero's if it is one of its best), adjusted by my
/// rules; the comp I play first in its tier.
fn tier_list(k: &Knowledge, s: &Snapshot, ctx: &Ctx, adv: &mut Advice) {
    let playing = adv.playing.as_ref().map(|p| p.comp_id.as_str());
    let mut ranked: Vec<(usize, RankedComp)> = comps::lobby_comps(k, ctx)
        .into_iter()
        .map(|c| {
            let verdict = rules::judge(&c.id, &k.rules, ctx);
            let (letter, avg) = comps::comp_tier(k, c, s.hero.as_deref(), verdict.bonus);
            let order = ["S", "A", "B", "C", "D", "?"].iter().position(|l| *l == letter).unwrap_or(5);
            (order, RankedComp { comp_id: c.id.clone(), letter, avg, verdict, playing: Some(c.id.as_str()) == playing })
        })
        .collect();
    ranked.sort_by(|a, b| {
        (a.0, !a.1.playing, a.1.avg).partial_cmp(&(b.0, !b.1.playing, b.1.avg)).unwrap_or(std::cmp::Ordering::Equal)
    });
    adv.ranked = ranked.into_iter().map(|(_, r)| r).collect();
    adv.hidden = comps::available_comps(&k.comps, ctx.races.as_deref())
        .into_iter()
        .filter(|c| rules::judge(&c.id, &k.rules, ctx).hidden)
        .map(|c| c.id.clone())
        .collect();
}

// ------------------------------------------------------------------------------- the reader thread
/// Where the reader sends its snapshots (the window, or the console).
pub type Sink = Box<dyn Fn(Arc<Snapshot>) + Send>;

/// Re-reads an old log up to a turn (0 = hero choice) and sends one snapshot (to look at the window
/// out of a game).
pub fn replay_snapshot(path: &std::path::Path, stop_turn: Option<i64>) -> std::io::Result<Snapshot> {
    let text = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&text);
    let mut parser = PowerLogParser::new(false);
    for line in text.lines() {
        parser.feed_line(line);
        let Some(g) = parser.game() else { continue };
        match stop_turn {
            Some(0) if g.hero_options().len() >= 4 => break,
            Some(t) if t > 0 && g.turn() >= t && g.shop().len() >= 3 => break,
            _ => {}
        }
    }
    let mut state = State::default();
    Ok(make_snapshot(parser.game_mut(), &mut state))
}

/// End of a game: learn the tribes of neutrals (BACON_SUBSET_* tags), write the game to history.jsonl,
/// queue it for sharing if the player said yes and it is new, and send the queue in the background.
fn game_over(game: &Game, log: Option<&std::path::Path>) {
    let mut subsets = pool::load_subsets();
    pool::learn_subsets(game, &cards::db(), &mut subsets);
    if let Err(e) = pool::save_subsets(&subsets) {
        crate::errlog::note("subsets.json", &e.to_string());
    }
    let k = knowledge::get();
    let Some(record) = history::game_record(game, &k, &history::log_date(log)) else { return };
    let added = match history::add_records(std::slice::from_ref(&record), &history::history_path()) {
        Ok(n) => n,
        Err(e) => {
            crate::errlog::note("history.jsonl", &e.to_string());
            0
        }
    };
    // Shared once: a game already in the history (app restarted after it) is not queued again.
    let share = Share::default();
    if added > 0 && share.add_game(game, &record) {
        send_in_background(share);
    }
}

/// Sends the sharing queue without blocking (one request for everything queued).
pub fn send_in_background(share: Share) {
    std::thread::spawn(move || {
        share.flush(&crate::share::http_send);
    });
}

/// Follows the live game until `stop` is set. Sends a snapshot when something changed.
pub fn run_live(sink: Sink, stop: Arc<AtomicBool>) {
    crate::background::lower_priority(); // reading and advice never take the CPU from the game
    let mut watcher = LogWatcher::new(None);
    let mut state = State::default();
    let mut last: Option<Arc<Snapshot>> = None;
    let mut status: Option<Status> = None;
    let mut done_game: Option<String> = None;
    let mut started = 0u64;
    let mut errors = 0;
    while !stop.load(Ordering::Relaxed) {
        let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let read = watcher.poll();
            if read {
                if watcher.parser.started != started {
                    started = watcher.parser.started;
                    state = State::default(); // a new game: fresh simulations
                }
                // Only once caught up: a long log read at start shows the end, not every step.
                if !watcher.caught_up() {
                    return;
                }
                // Not while catching up: only the end of the file is the game going on.
                if let Some(g) = watcher.parser.game() {
                    let id = format!("{}#{}", started, g.start_time);
                    if g.is_battlegrounds() && g.is_over() && done_game.as_ref() != Some(&id) {
                        done_game = Some(id);
                        game_over(g, watcher.path.as_deref());
                    }
                }
                let mut snap = make_snapshot(watcher.parser.game_mut(), &mut state);
                snap.log_status = Some(watcher.status);
                if last.as_deref() != Some(&snap) {
                    let snap = Arc::new(snap);
                    last = Some(snap.clone());
                    sink(snap);
                }
            } else if Some(watcher.status) != status && last.as_ref().is_none_or(|l| !l.in_game) {
                // Nothing to read: say why (folder not found, no Power.log yet...).
                let snap = Arc::new(Snapshot {
                    log_status: Some(watcher.status),
                    logs_dir: watcher.logs_dir.clone(),
                    data_errors: knowledge::get().errors.clone(),
                    ..Default::default()
                });
                last = Some(snap.clone());
                sink(snap);
            }
            status = Some(watcher.status);
        }));
        match step {
            Ok(()) => errors = 0,
            Err(_) => {
                // Written to error.log by the panic hook.
                errors += 1;
                if errors >= 5 {
                    watcher = LogWatcher::new(None);
                    state = State::default();
                    last = None;
                    errors = 0;
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
        if !watcher.caught_up() && watcher.status == Status::Ok {
            continue; // more to read right away
        }
        // 4 reads a second: a change (power, shop) shows quickly.
        std::thread::sleep(Duration::from_millis(250));
    }
}
