"""Partage opt-in : rien sans accord, résumé limité aux champs prévus, envoi groupé, effacement."""
import json
from types import SimpleNamespace

import pytest

from bgtracker import share

RECORD = {"hero": "Arch-Villain Rafaam", "deity": None, "races": ["UNDEAD", "PIRATE"], "duos": False,
          "placement": 3, "comp": "undead-apm", "board": ["x"], "date": "2026-11-02", "time": "12:00:41"}


class Ent:
    def __init__(self, cid, atk, hp, golden=False):
        self.card_id, self.is_golden, self.tags = cid, golden, {"ATK": atk, "HEALTH": hp}

    def get(self, tag, default=None):
        return self.tags.get(tag, default)


GAME = SimpleNamespace(
    turn=6, my_health=-3, conceded=False, actions_by_turn={1: 2, 2: 1, 3: 6, 5: 4, 6: 9},
    my_board=[Ent("BG25_008", 12, 10), Ent("", 1, 1)],
    fights=[{"turn": 5, "my_tier": 3, "opp_tier": 3, "my_hp": 30, "result": "W", "damage": 4, "opponent": 7,
             "my_board": [{"card_id": "BG25_008", "attack": 4, "health": 2, "golden": False}],
             "board": [{"card_id": "BG26_135", "attack": 3, "health": 3, "golden": True}]}])


@pytest.fixture(autouse=True)
def home(tmp_path, monkeypatch):
    for name in ("STATE_PATH", "QUEUE_PATH", "LAST_PATH", "STATS_PATH"):
        monkeypatch.setattr(share, name, tmp_path / getattr(share, name).name)
    monkeypatch.setattr(share, "API_URL", "https://api.example.workers.dev")
    return tmp_path


def test_nothing_is_queued_until_the_player_says_yes():
    assert not share.asked() and not share.add_game(GAME, RECORD)
    share.set_enabled(False)
    assert share.asked() and not share.add_game(GAME, RECORD) and share.pending() == []


def test_summary_has_only_the_listed_fields():
    s = share.game_summary(GAME, RECORD)
    assert set(s) == {"v", "app", "mode", "hero", "deity", "comp", "races", "place", "board", "fights",
                      "turns", "actions", "end_hp", "conceded"}
    assert s["actions"] == [2, 1, 6, 0, 4, 9] and s["end_hp"] == -3 and s["conceded"] is False
    assert s["board"] == [{"id": "BG25_008", "a": 12, "h": 10, "g": False}]      # hidden card dropped
    assert set(s["fights"][0]) == {"t", "tier", "otier", "hp", "r", "d", "me", "opp"}  # no opponent id
    text = json.dumps(s)
    assert "12:00:41" not in text and "2026-11-02" not in text


def test_flush_sends_games_and_feedback_in_one_request():
    share.set_enabled(True)
    share.add_game(GAME, RECORD)
    share.add_game(GAME, RECORD)
    share.add_feedback("idea", "Dark mode please")
    calls = []
    assert share.flush(send=lambda m, url, body=None: calls.append((m, url, body)) or 200) == 3
    assert len(calls) == 1 and len(calls[0][2]["games"]) == 2 and len(calls[0][2]["feedback"]) == 1
    assert share.pending() == []


def test_feedback_is_sent_even_without_game_sharing_and_kept_offline():
    share.add_feedback("bug", "Panel overlaps the leaderboard")

    def offline(*a, **k):
        raise OSError("no network")
    assert share.flush(send=offline) == 0 and len(share.pending()) == 1
    assert share.flush(send=lambda m, url, body=None: 200) == 1


def test_bad_feedback_refused():
    with pytest.raises(ValueError):
        share.add_feedback("spam", "hello")
    with pytest.raises(ValueError):
        share.add_feedback("idea", " ")


def test_delete_remote_forgets_the_install_id():
    share.set_enabled(True)
    old = share.install_id()
    calls = []
    assert share.delete_remote(send=lambda m, url, body=None: calls.append((m, url)) or 200)
    assert calls == [("DELETE", "https://api.example.workers.dev/v1/installs/" + old)]
    assert not share.enabled() and "install" not in share.state()


def test_feedback_typed_during_a_send_is_kept():
    share.add_feedback("idea", "first idea")

    def slow_send(method, url, body=None):
        share.add_feedback("bug", "typed while sending")    # arrive pendant l'envoi
        return 200
    assert share.flush(send=slow_send) == 1
    assert [i["data"]["text"] for i in share.pending()] == ["typed while sending"]
