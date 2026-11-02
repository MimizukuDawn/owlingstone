"""Tests du panneau Lobby (combats lus dans le log) et des mises à jour (version, données à part)."""
import pytest

from bgtracker.cards import get_db
from bgtracker.hud import ordinal, today_games
from bgtracker.paths import migrate
from bgtracker.update import newer_version, parse, source_version, whats_new
from tests.test_powerlog import game1  # noqa: F401  (fixture partagée, utilisée en paramètre)


def _names(fight):
    return [get_db().base(m["card_id"]).name for m in fight["board"]]


def test_game1_fights(game1):  # noqa: F811
    # 10 combats (le dernier m'élimine), adversaires lus dans NEXT_OPPONENT_PLAYER_ID.
    assert [f["result"] for f in game1.fights] == list("TTTWWLLLLL")
    t4 = game1.fights[3]
    assert game1.hero_of(t4["opponent"]).name == "Forest Lord Cenarius"
    assert _names(t4) == ["Scarlet Survivor", "Tad", "Tad"]
    assert t4["damage"] == 4
    last = game1.fights[-1]
    assert last["turn"] == 10 and last["result"] == "L" and last["damage"] == 15
    assert len(last["board"]) == 7 and any(m["golden"] for m in last["board"])


def test_game1_lobby_players(game1):  # noqa: F811
    chromie = game1.hero_of(6)
    assert chromie.name == "Time Twister Chromie"
    assert (chromie.get("PLAYER_TECH_LEVEL"), chromie.get("PLAYER_TRIPLES"), game1.player_health(6)) == (5, 2, 32)
    assert game1.last_fight_against(6)["turn"] == 6


def test_today_games_and_ordinal():
    recs = [{"date": "2026-10-29", "time": "20:00:00", "placement": 3},
            {"date": "2026-10-28", "time": "21:00:00", "placement": 1},
            {"date": "2026-10-29", "time": "19:00:00", "placement": 6}]
    assert [r["placement"] for r in today_games(recs, "2026-10-29")] == [6, 3]
    assert [ordinal(n) for n in (1, 2, 3, 4)] == ["1st", "2nd", "3rd", "4th"]


def test_version_compare(tmp_path):
    (tmp_path / "bgtracker").mkdir()
    (tmp_path / "bgtracker" / "__init__.py").write_text('__version__ = "1.10.0"\n', encoding="utf-8")
    assert source_version(tmp_path) == "1.10.0"
    assert parse("1.10.0") > parse("1.9.3")
    assert newer_version("1.9.0", tmp_path) is None     # lancé avec Python (pas un exe) : jamais


def test_whats_new(tmp_path):
    log = tmp_path / "CHANGELOG.md"
    log.write_text("# t\n\n## 1.2.0\n- a\n- b\n\n## 1.1.0\n- old\n", encoding="utf-8")
    assert whats_new("1.2.0", log) == ["a", "b"]
    assert whats_new("9.9.9", log) == []


def test_migrate_copies_once_without_overwriting(tmp_path):
    old, user = tmp_path / "data", tmp_path / "user"
    (old / "tiles").mkdir(parents=True)
    (old / "tiles" / "X.png").write_bytes(b"png")
    (old / "history.jsonl").write_text("old\n", encoding="utf-8")
    user.mkdir()
    (user / "history.jsonl").write_text("mine\n", encoding="utf-8")
    assert migrate(old, user) == ["tiles"]
    assert (user / "history.jsonl").read_text(encoding="utf-8") == "mine\n"   # jamais écrasé
    assert (user / "tiles" / "X.png").exists() and (old / "history.jsonl").exists()
    assert migrate(old, user) == []                                          # une seule fois


@pytest.mark.parametrize("name", ["comps.yaml", "heroes.yaml", "rules.yaml"])
def test_app_data_stays_in_project(name):
    from bgtracker.paths import DATA_DIR, USER_DIR
    assert (DATA_DIR / name).exists() and DATA_DIR != USER_DIR


def test_card_list_marks_owned_and_shop():
    from bgtracker.comps import load_comps
    from bgtracker.hud import card_list
    comp = next(c for c in load_comps() if c.id == "aberration-deathrattle")
    a, b = comp.core
    text = card_list(comp.core, owned={a.id}, shop={b.id})
    assert "✓ " + a.name in text and "● " + b.name in text


def test_game1_tavern_spell_bonus(game1):  # noqa: F811
    # Fin de partie 1 : mes sorts de taverne donnaient +0/+1 (tag TAVERN_SPELL_HEALTH_INCREASE sur moi).
    assert game1.my_bonuses == {"spell": (0, 1), "gem": (1, 1)}


def test_game1_hero_power(game1):  # noqa: F811
    assert [e.name for e in game1.my_hero_powers] == ["Choose Your Champion"]


def test_side_panels_stay_in_the_edges():
    from bgtracker.ui import side_panel_geometry, PANEL_MIN_WIDTH
    # 16:9 1920x1080 : la zone 4:3 va de x=240 à x=1680
    x, y, w, h = side_panel_geometry((0, 0, 1920, 1080), "left")
    assert x == 0 and x + w < 240 and w >= PANEL_MIN_WIDTH
    x, y, w, h = side_panel_geometry((0, 0, 1920, 1080), "right")
    assert x + w == 1920 and x > 1680
    # réglage de l'utilisateur
    assert side_panel_geometry((0, 0, 1920, 1080), "left", -20)[2] == side_panel_geometry((0, 0, 1920, 1080), "left")[2] - 20
    assert side_panel_geometry((0, 0, 1920, 1080), "left", -500)[2] == PANEL_MIN_WIDTH


def test_top_overlay_centered_at_top():
    from bgtracker.hud import CombatOverlay, top_overlay_geometry
    x, y, w, h = top_overlay_geometry((0, 0, 1920, 1080))
    assert abs((x + w / 2) - 960) <= 1 and y < 20 and w == CombatOverlay.WIDTH


def test_folded_section_hides_until_next_header():
    import os
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtWidgets import QApplication, QLabel, QVBoxLayout, QWidget
    from bgtracker import hud
    app = QApplication.instance() or QApplication([])  # noqa: F841
    page = QWidget()
    lay = QVBoxLayout(page)
    a, b = hud.section("Buy"), hud.section("Turn plan · 5 gold")
    x, y = QLabel("x"), QLabel("y")
    for w in (a, x, b, y):
        lay.addWidget(w)
    hud.FOLDED.clear()
    hud.FOLDED.add("buy")
    hud.apply_folds(lay)
    assert x.isHidden() and not y.isHidden() and b.key == "turn plan"
    hud.FOLDED.clear()
    hud.apply_folds(lay)
    assert not x.isHidden()


def test_comp_tier_letters():
    from bgtracker.comps import Comp, comp_tier
    c = Comp("x", "X", [], [], [], stats={"avg": 3.45})
    assert comp_tier(c, table={})[0] == "A"
    assert comp_tier(c, bonus=3, table={})[0] == "S"          # règle prefer_if de 3 points
    assert comp_tier(Comp("y", "Y", [], [], []), table={})[0] == "?"
