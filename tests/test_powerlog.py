"""Tests du lecteur de log sur de vraies parties (logs locaux dans testdata/, jamais publiés).

- test_game1_* : partie du 2026-10-26 copiée dans testdata/ (valeurs vérifiées dans le log).
- test_all_local_logs : rejoue tous les Power_old.log du dossier du jeu (ignoré s'il n'existe pas)
  et vérifie qu'on trouve bien 5 races sûres dans chaque partie BG terminée.
"""
from pathlib import Path

import pytest

from bgtracker.powerlog import PowerLogParser
from bgtracker.watcher import DEFAULT_LOGS_DIR, LogWatcher

GAME1 = Path(__file__).parent.parent / "testdata" / "bg_game1.log"


def _parse(path):
    p = PowerLogParser()
    with open(path, encoding="utf-8", errors="replace") as f:
        p.feed_lines(f)
    return p


@pytest.fixture(scope="module")
def game1():
    if not GAME1.exists():
        pytest.skip("testdata/bg_game1.log absent")
    return _parse(GAME1).game


def test_game1_basics(game1):
    assert game1.is_battlegrounds
    assert game1.my_hero.name == "Thorim, Stormlord"
    assert game1.tavern_tier == 6
    assert game1.is_over
    assert game1.my_placement == 6


def test_game1_races(game1):
    # Les 5 races du lobby (= tags BACON_SUBSET_* du log) ; Démon/Pirate/Mort-vivant ne sont venus
    # que via des serviteurs à double race et ne doivent PAS être comptés.
    assert sorted(game1.available_races) == ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]
    assert game1.races_complete


def test_game1_lobby_has_8_heroes(game1):
    heroes = game1.lobby_heroes
    assert len(heroes) == 8
    assert sorted(h.get("PLAYER_LEADERBOARD_PLACE") for h in heroes) == list(range(1, 9))


def test_watcher_reads_partial_lines(tmp_path):
    """Le jeu écrit parfois une ligne en deux fois : elle ne doit être lue qu'une fois complète."""
    folder = tmp_path / "Hearthstone_2026_01_01_00_00_00"
    folder.mkdir()
    log = folder / "Power.log"
    w = LogWatcher(logs_dir=tmp_path)
    log.write_text("D 00:00:00.0 GameState.DebugPrintPower() - CREATE_GAME\n"
                   "D 00:00:00.0 GameState.DebugPrintPower() -     GameEntity EntityID=1\n"
                   "D 00:00:00.0 GameState.DebugPrintPower() - TAG_CHANGE Entity=1 tag=TU", encoding="utf-8")
    w.poll()
    assert w.game.game_entity.get("TURN") is None      # ligne incomplète : pas encore lue
    with open(log, "a", encoding="utf-8") as f:
        f.write("RN value=3 \n")
    w.poll()
    assert w.game.game_entity.get("TURN") == 3


def _local_logs():
    if not DEFAULT_LOGS_DIR.exists():
        return []
    return sorted(DEFAULT_LOGS_DIR.glob("Hearthstone_*/Power*.log"))


@pytest.mark.slow
@pytest.mark.parametrize("path", _local_logs(), ids=lambda p: p.parent.name + "/" + p.name)
def test_all_local_logs(path):
    games = [g for g in _parse(path).games if g.is_battlegrounds and g.is_over]
    for g in games:
        assert g.races_complete, f"races incomplètes : {g.available_races} / {g.possible_races}"
        assert g.my_placement in range(1, 9)
