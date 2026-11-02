"""Tests des Trinkets : lecture de l'offre dans le log, classement de l'offre, aide avant l'offre."""
from bgtracker.cards import get_db
from bgtracker.comps import Comp
from bgtracker.game import Game
from bgtracker.trinkets import IN_TYPE, load_trinkets, next_offer_hint, race_counts, rank_offer


def card(name, trinket_kind=None):
    db = get_db()
    if trinket_kind:
        t = next(t for t in load_trinkets().values() if t["name"] == name and t["kind"] == trinket_kind)
        return db.get(t["id"])
    found = [c for c in db.by_id.values() if c.name == name and c.tier and c.id.startswith("BG")]
    return next((c for c in found if c.is_pool_minion), found[0])


def _game():
    g = Game()
    g.players = {2: {"entity_id": 20, "is_real_account": True}, 9: {"entity_id": 90, "is_real_account": False}}
    g.game_entity_id = 1
    g.create_entity(1, "").tags["TURN"] = 11                 # tour 6, recrutement
    for eid in (20, 90):
        g.create_entity(eid, "")
    return g


def _ent(g, eid, card_id, **tags):
    e = g.create_entity(eid, card_id)
    e.tags.update(tags)
    return e


def test_offer_read_from_log_state():
    """Offre = Trinkets en SETASIDE créés par mon bouton "Lesser Trinket" ; prix réduit lu sur COST ; le choisi
    devient le bouton (CHANGE_ENTITY), les autres partent en REMOVEDFROMGAME."""
    g = _game()
    _ent(g, 392, "BG30_Trinket_1st", CONTROLLER=2, ZONE="PLAY", CARDTYPE="BATTLEGROUND_TRINKET")
    _ent(g, 3214, "BG30_MagicItem_411", CONTROLLER=2, ZONE="SETASIDE", CARDTYPE="BATTLEGROUND_TRINKET",
         BACON_TRINKET=1, CREATOR=392)                           # Hoggy Bank, pas de COST : prix de la carte
    _ent(g, 3215, "BG36_MagicItem_202", CONTROLLER=2, ZONE="SETASIDE", CARDTYPE="BATTLEGROUND_TRINKET",
         BACON_TRINKET=1, CREATOR=392, COST=2)                   # Warcry Totem pivot : 4 -> 2
    _ent(g, 5000, "BG32_MagicItem_366", CONTROLLER=2, ZONE="SETASIDE", CARDTYPE="BATTLEGROUND_TRINKET",
         BACON_TRINKET=1, CREATOR=0)                             # copie vue en combat : pas une offre
    offer = [(e.name, cost) for e, cost in g.trinket_offer]
    assert offer == [("Hoggy Bank", 0), ("Warcry Totem", 2)]
    assert g.my_trinkets == []
    # Choix : le bouton devient le Trinket, l'offre disparaît
    g.entities[392].card_id = "BG30_MagicItem_411"
    for eid in (3214, 3215):
        g.entities[eid].tags["ZONE"] = "REMOVEDFROMGAME"
    assert g.trinket_offer == []
    assert [e.name for e in g.my_trinkets] == ["Hoggy Bank"]


def test_rank_prefers_my_tribe_and_comp_cards():
    quil = [card("Tusked Camper"), card("Snare Trapper"), card("Gilded Gargoyle")]
    offer = [(card("Hoggy Bank", "lesser"), 0), (card("Ornate Clock", "lesser"), 0),
             (card("Warcry Totem", "lesser"), 2)]
    picks = rank_offer(offer, quil, gold=10)
    assert picks[0].card.name == "Hoggy Bank"
    totem = next(p for p in picks if p.card.name == "Warcry Totem")
    assert any("pivot" in r for r in totem.reasons) and any("Dragon/Murloc" in r for r in totem.reasons)
    assert totem.score > 4.0                       # pas de Dragon / Murloc ni de Cri de guerre : malus


def test_portrait_giving_a_core_card_wins():
    beatboxer = card("Polarizing Beatboxer")
    comp = Comp(id="mech", name="Mech Test", races=["MECHANICAL"], core=[beatboxer], addons=[])
    offer = [(card("Beatboxer Portrait", "greater"), 6), (card("Surveyor Portrait", "greater"), 3)]
    mechs = [card("Annoy-o-Tron"), card("Annoy-o-Tron"), card("Annoy-o-Tron")] if _has("Annoy-o-Tron") else []
    picks = rank_offer(offer, mechs, targets=[(comp, 1.0)], gold=12)
    assert picks[0].card.name == "Beatboxer Portrait"
    assert any("core of Mech Test" in r for r in picks[0].reasons)


def test_unaffordable_goes_last():
    offer = [(card("Weighted Gauntlet", "lesser"), 3), (card("Ornate Clock", "lesser"), 0)]
    picks = rank_offer(offer, [], gold=2)
    assert picks[0].card.name == "Ornate Clock" and not picks[1].affordable


def test_hint_before_offer():
    assert next_offer_hint(6, []) is None and next_offer_hint(7, []) is None
    board = [card("Tusked Camper"), card("Snare Trapper"), card("Gilded Gargoyle")]
    h = next_offer_hint(5, board)
    assert h["kind"] == "lesser" and h["need"] == IN_TYPE["lesser"] == 2
    assert h["unlocked"] == ["QUILBOAR"] and "UNDEAD" in h["close"]
    assert any(line.startswith("+1 Undead unlocks: Eternal Portrait") for line in h["lines"])
    h = next_offer_hint(8, board)                  # tour 9 : il en faut 3
    assert h["kind"] == "greater" and h["unlocked"] == [] and "QUILBOAR" in h["close"]


def test_race_counts_amalgam_counts_everywhere():
    n = race_counts([card("Tusked Camper")] + [c for c in get_db().pool_minions(duos=True) if "ALL" in c.races][:1])
    assert n["QUILBOAR"] == 2 and n["BEAST"] == 1


def test_trinket_table_is_current_pool():
    table = load_trinkets()
    assert len(table) >= 250
    assert all(get_db().get(i) for i in table)
    assert {t["kind"] for t in table.values()} == {"lesser", "greater"}


def _has(name):
    return any(c.name == name for c in get_db().pool_minions(duos=True))
