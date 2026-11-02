"""Tests de la base de comps et de la détection "ce que je joue"."""
import pytest

from bgtracker.cards import get_db
from bgtracker.comps import available_comps, board_profile, load_comps, lobby_comps, match_comps
from bgtracker.rules import Rule
from tests.test_powerlog import GAME1, _parse


def _cards(*names):
    db = get_db()
    pool = db.pool_minions() + db.pool_spells()
    return [next(c for c in pool if c.name == n) for n in names]


def test_comps_file_valid():
    """Chaque nom de carte de comps.yaml existe dans le pool Solo actuel (sinon load_comps lève une erreur)."""
    comps = load_comps()
    assert len(comps) >= 25
    assert len({c.id for c in comps}) == len(comps)  # ids uniques
    assert all(c.stats.get("avg") for c in comps if c.id != "aberration-apm")


def test_no_duos_or_naga_cards_in_solo_pool():
    db = get_db()
    names = {c.name for c in db.pool_minions() + db.pool_spells()}
    assert "Voidpriest Cloner" not in names and "C'Thrax Wrecker" not in names   # Duos seulement
    assert "Groundbreaker" not in names                                           # Naga : hors du pool
    assert "Voidpriest Cloner" in {c.name for c in db.pool_minions(duos=True)}
    assert any(set(c.races) == {"NAGA", "DRAGON"} for c in db.pool_minions())      # double race : reste


def test_comps_filtered_by_lobby_races():
    comps = load_comps()
    ids = {c.id for c in available_comps(comps, ["ABERRATION", "DRAGON", "MURLOC", "MECHANICAL", "BEAST"])}
    assert {"aberration-discard", "aberration-deathrattle", "neutral-tea-set"} <= ids
    assert "aberration-apm" not in ids      # pas de Pirates dans le lobby
    assert "pirate-discover" not in ids


def test_board_detection_core_cards_win():
    comps = load_comps()
    board = _cards("Mysterious K'Thir", "Nightmare Corroder", "Cutthroat K'Thir")
    best = match_comps(comps, board)[0]
    assert best.comp.id == "aberration-discard"
    assert "Harbinger Aph'lass" in [c.name for c in best.missing_core]


def test_hand_counts_less_than_board():
    comps = load_comps()
    board = _cards("Auto Reveille", "Spark Snapper")                            # 2 core de Mech APM
    hand = _cards("Resourceful Robot", "Utility Drone", "Drakkari Enchanter")    # 3 core de Volumizer
    assert match_comps(comps, board, hand)[0].comp.id == "mech-apm"
    assert match_comps(comps, board + hand)[0].comp.id == "mech-volumizer"


def test_rules_hide_and_break_ties():
    comps = load_comps()
    board = _cards("Nightmare Corroder")          # add-on de Discard ET de Deathrattle : égalité
    lobby = ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]
    prefer = [Rule(comps=["aberration-deathrattle"], effect="prefer_if", cond={"deity": "C'Thun"})]
    assert match_comps(comps, board, races=lobby, rules=prefer, deity="C'Thun")[0].comp.id == "aberration-deathrattle"
    prefer = [Rule(comps=["aberration-discard"], effect="prefer_if", cond={"deity": "C'Thun"})]
    assert match_comps(comps, board, races=lobby, rules=prefer, deity="C'Thun")[0].comp.id == "aberration-discard"
    hide = [Rule(comps=["aberration-deathrattle"], effect="never_if", cond={"lobby_has": ["BEAST"]})]
    board = _cards("Faceless Converter")          # core de Deathrattle
    assert "aberration-deathrattle" not in {m.comp.id for m in match_comps(comps, board, races=lobby, rules=hide)}
    # Races pas encore sûres : la règle ne cache rien.
    assert "aberration-deathrattle" in {m.comp.id for m in match_comps(comps, board, races=None, rules=hide)}


def test_rule_bonus_does_not_beat_a_real_match():
    """Plateau du tour 9 de game1 : Spark Snapper (core Mech APM) + Shadow of Doubt (simple add-on).
    Même avec C'Thun préféré pour Deathrattle, la comp reconnue reste Mech APM."""
    comps = load_comps()
    board = _cards("Spark Snapper", "The Shadow of Doubt", "Forest Rover", "Futurefin")
    prefer = [Rule(comps=["aberration-deathrattle"], effect="prefer_if", cond={"deity": "C'Thun"})]
    best = match_comps(comps, board, rules=prefer, deity="C'Thun", min_confidence=15)[0]
    assert best.comp.id == "mech-apm"


def test_lobby_comps_sorted_by_preference_then_placement():
    comps = load_comps()
    lobby = ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]
    names = [c.id for c in lobby_comps(comps, lobby)]
    assert names[0] in ("dragon-shield", "neutral-tea-set")    # 3.16, meilleures moyennes
    assert names[-1] == "mech-glambot"                           # 4.23, la plus faible
    prefer = [Rule(comps=["mech-glambot"], effect="prefer_if", cond={})]
    assert lobby_comps(comps, lobby, rules=prefer)[0].id == "mech-glambot"


def test_race_fit_alone_is_not_a_comp():
    """Des Aberrations sans aucune carte d'une comp : on ne prétend pas savoir ce que je joue."""
    assert match_comps(load_comps(), _cards("Brain Rotter", "Underrot Spawn")) == []


def test_board_profile_counts_races_and_keywords():
    p = board_profile(_cards("Ghastcoiler", "Sewer Lord", "Mummifier"))
    assert p["races"] == {"BEAST": 2, "UNDEAD": 1}
    assert p["keywords"]["Deathrattle"] == 3


def test_game1_final_board_is_mech():
    """Fin de la partie réelle du 26/10 : 2 Spark Snapper + Auto Accelerator -> Mech APM."""
    if not GAME1.exists():
        pytest.skip("testdata/bg_game1.log absent")
    g = _parse(GAME1).game
    db = get_db()
    board = [db.base(e.card_id) for e in g.my_board]
    hand = [db.base(e.card_id) for e in g.my_hand]
    assert match_comps(load_comps(), board, hand, g.available_races)[0].comp.id == "mech-apm"
    assert g.deity == "C'Thun"


def test_keyword_filter_and_dark_paradox_apart():
    from bgtracker.pool import available_keywords, tier_list
    lobby = ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]
    assert "Deathrattle" in available_keywords(lobby)
    sections = dict(tier_list(5, lobby, keyword="Deathrattle"))
    names = {c.name for cards in sections.values() for c in cards}
    assert {"Ghastcoiler", "Titus Rivendare"} <= names     # Titus cite Deathrattle : il compte
    assert "SPELL" not in sections                          # filtre actif : pas de sorts
    # Dark Paradox (tier différent à chaque partie) : listé à part, à chaque tier.
    for tier in (1, 4):
        assert [c.name for c in dict(tier_list(tier, lobby))["VARIES"]] == ["Dark Paradox"]


def test_tier7_reachable_only_with_a_way_in():
    from bgtracker.pool import tier7_reachable
    lobby = ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]
    assert not tier7_reachable(lobby)
    assert tier7_reachable(lobby[:4] + ["PIRATE"])          # Greedy Conniver
    assert tier7_reachable(lobby, seen_tier7=True)          # ex. Futurefin donné par un trinket
    assert tier7_reachable(None)                             # races pas encore connues
