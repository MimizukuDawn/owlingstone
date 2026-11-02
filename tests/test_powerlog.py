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


def test_watcher_finds_a_new_game_folder_and_reads_in_chunks(tmp_path, monkeypatch):
    """Dossier recherché toutes les RESCAN secondes seulement ; gros fichier lu par blocs sans couper
    une ligne ni un caractère."""
    import bgtracker.watcher as watcher
    clock = [0.0]
    w = LogWatcher(logs_dir=tmp_path, clock=lambda: clock[0])
    assert not w.poll() and w.status == "no_log"
    folder = tmp_path / "Hearthstone_2026_01_01_00_00_00"
    folder.mkdir()
    lines = "".join(f"D 00:00:00.0 GameState.DebugPrintPower() - TAG_CHANGE Entity=1 tag=TURN value={i} é\n"
                    for i in range(1, 50))
    (folder / "Power.log").write_text("D 00:00:00.0 GameState.DebugPrintPower() - CREATE_GAME\n"
                                      "D 00:00:00.0 GameState.DebugPrintPower() -     GameEntity EntityID=1\n"
                                      + lines, encoding="utf-8")
    assert not w.poll()                    # pas encore recherché (moins de RESCAN secondes)
    clock[0] += watcher.RESCAN
    monkeypatch.setattr(watcher, "CHUNK", 37)
    while w.poll():
        pass
    assert w.status == "ok" and w.game.game_entity.get("TURN") == 49


def test_logs_dir_found_outside_default_path(tmp_path):
    from bgtracker import hslogs
    settings = tmp_path / "settings.json"
    install = tmp_path / "Games" / "Hearthstone"
    (install / "Logs").mkdir(parents=True)
    none = lambda: None
    assert hslogs.find_logs_dir(settings_path=settings, running=lambda: install, registry=none) == install / "Logs"
    assert hslogs.find_logs_dir(settings_path=settings, running=none, registry=lambda: install) == install / "Logs"
    settings.write_text('{"hearthstone_dir": "%s"}' % (install / "Logs").as_posix(), encoding="utf-8")
    assert hslogs.find_logs_dir(settings_path=settings, running=none, registry=none) == install / "Logs"


def test_log_config_gets_power_section_once(tmp_path):
    from bgtracker import hslogs
    cfg = tmp_path / "Blizzard" / "Hearthstone" / "log.config"
    assert hslogs.ensure_log_config(cfg) == "written"
    assert "[Power]" in cfg.read_text(encoding="utf-8")
    assert hslogs.ensure_log_config(cfg) == "ok"
    other = tmp_path / "other.config"
    other.write_text("[Zone]\nLogLevel=1", encoding="utf-8")
    assert hslogs.ensure_log_config(other) == "written"
    text = other.read_text(encoding="utf-8")
    assert text.startswith("[Zone]") and text.count("[Power]") == 1


def _game_lines(turn, filler=0):
    pad = "".join("D 00:00:00.0 PowerTaskList.DebugPrintPower() -     filler line\n" for _ in range(filler))
    return ("D 00:00:00.0 GameState.DebugPrintPowerList() - Count=1\n"
            "D 00:00:00.0 GameState.DebugPrintPower() - CREATE_GAME\n"
            "D 00:00:00.0 GameState.DebugPrintPower() -     GameEntity EntityID=1\n"
            f"D 00:00:00.0 GameState.DebugPrintPower() - TAG_CHANGE Entity=1 tag=TURN value={turn}\n" + pad)


@pytest.mark.parametrize("block", [7, 40, 64, 1 << 20])
def test_last_game_offset_finds_the_last_create_game(tmp_path, block):
    from bgtracker.watcher import last_game_offset
    log = tmp_path / "Power.log"
    text = _game_lines(1, 30) + _game_lines(2, 30) + _game_lines(3, 5)
    log.write_bytes(text.encode())
    start = last_game_offset(log, block=block)
    assert text.encode()[start:].startswith(b"D 00:00:00.0 GameState.DebugPrintPower() - CREATE_GAME")
    assert text.encode()[start:].count(b"CREATE_GAME") == 1
    log.write_bytes(b"no game here\n" * 10)
    assert last_game_offset(log, block=block) == 0


def test_watcher_skips_old_games_of_a_big_log(tmp_path, monkeypatch):
    """Bug 1.9.0 : au démarrage, un gros Power.log (toute la soirée) était relu depuis le début par blocs
    et les vieilles parties défilaient à l'écran. On commence à la dernière partie ; behind dit si on
    rattrape encore du retard."""
    import bgtracker.watcher as watcher
    folder = tmp_path / "Hearthstone_2026_01_01_00_00_00"
    folder.mkdir()
    log = folder / "Power.log"
    log.write_text(_game_lines(12, 50) + _game_lines(8, 50) + _game_lines(3, 50), encoding="utf-8")
    seen = []
    w = LogWatcher(logs_dir=tmp_path, on_new_game=seen.append)
    monkeypatch.setattr(watcher, "CHUNK", 300)
    assert w.poll() and w.behind
    while w.poll():
        pass
    assert not w.behind and len(seen) == 1 and w.game.game_entity.get("TURN") == 3
    with open(log, "a", encoding="utf-8") as f:      # partie suivante écrite pendant que l'appli tourne
        f.write(_game_lines(1))
    w.poll()
    assert len(seen) == 2 and w.game.game_entity.get("TURN") == 1
