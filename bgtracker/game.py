"""État d'une partie : entités, tags, et lecture "Battlegrounds" de cet état.

Vocabulaire :
- Entité : tout objet de la partie (la partie elle-même, un joueur, un héros, un serviteur,
  un pouvoir, un enchantement...). Chaque entité a un numéro (id) et des tags.
- Tag : une propriété "nom=valeur", ex. ZONE=PLAY (sur le plateau), CONTROLLER=4 (à qui),
  PLAYER_TECH_LEVEL=3 (tier de taverne du héros), CARDRACE=BEAST (race).
- Bob : l'aubergiste. Ses serviteurs en ZONE=PLAY hors combat = la boutique (le "shop").
"""
from collections import Counter  # compter combien de fois on voit chaque race

from .cards import get_db
from .combat import snapshot_minion

BONUS_TAGS = ("TAVERN_SPELL_ATTACK_INCREASE", "TAVERN_SPELL_HEALTH_INCREASE",
              "BACON_BLOODGEMBUFFATKVALUE", "BACON_BLOODGEMBUFFHEALTHVALUE")
LOBBY_RACE_COUNT = 5  # nombre de races par partie (règle actuelle du mode) ⚠️ à revérifier si Blizzard change


def _to_value(raw):
    """Les valeurs sont soit des nombres ("3"), soit des noms ("PLAY") : on convertit les nombres."""
    try:
        return int(raw)
    except ValueError:
        return raw


class Entity:
    def __init__(self, game, eid, card_id):
        self.game = game        # pour prévenir la partie quand un tag change
        self.id = eid
        self.card_id = card_id  # "" tant que la carte est cachée
        self.tags = {}

    def get(self, tag, default=None):
        return self.tags.get(tag, default)

    def set_tag(self, tag, raw_value):
        self.tags[tag] = _to_value(raw_value)
        self.game._on_tag(self, tag)  # la partie met à jour ses déductions (races, etc.)

    @property
    def card(self):
        return get_db().base(self.card_id) if self.card_id else None

    @property
    def name(self):
        c = self.card
        return c.name if c else (self.card_id or f"#{self.id}")

    @property
    def is_minion(self):
        return self.get("CARDTYPE") == "MINION"

    @property
    def is_golden(self):
        return bool(self.get("PREMIUM"))  # PREMIUM=1 : version dorée (triple)

    def __repr__(self):
        return f"<{self.name} id={self.id}>"


class Game:
    def __init__(self):
        self.entities = {}        # id -> Entity
        self.players = {}         # PlayerID -> {"entity_id": .., "is_real_account": ..}
        self.player_names = {}    # PlayerID -> nom affiché
        self.game_type = None     # ex. "GT_BATTLEGROUNDS"
        self.start_time = ""      # heure de CREATE_GAME dans le log, ex. "12:00:41"
        self.game_entity_id = None
        self.shop_race_counts = Counter()  # race -> nb de serviteurs vus dans la boutique
        self._seen_in_shop = set()         # ids déjà comptés (ne pas compter 2 fois)
        self._dual_race_seen = []          # paires de races des serviteurs à double race
        self.subset_tags_seen = set()      # ids d'entités portant un tag BACON_SUBSET_*
        self.fights = []                   # mes combats, dans l'ordre (voir _start_fight)
        self._fight = None                 # combat en cours d'enregistrement
        self.deity_seen = {}               # PLAYER_ID -> (attaque, vie) de sa Déité la dernière fois vue
        self.bonus_tags = {}               # derniers compteurs vus sur mes entités (voir my_bonuses)

    # ------------------------------------------------------------- remplissage
    def create_entity(self, eid, card_id):
        ent = self.entities.get(eid)
        if ent is None:
            ent = self.entities[eid] = Entity(self, eid, card_id)
        elif card_id:
            ent.card_id = card_id
        return ent

    def tag_change(self, ent, tag, raw_value):
        ent.set_tag(tag, raw_value)

    def entity_by_player_name(self, name):
        for pid, pname in self.player_names.items():
            if pname == name and pid in self.players:
                return self.entities.get(self.players[pid]["entity_id"])
        return None

    def _on_tag(self, ent, tag):
        """Appelé à chaque tag reçu. Sert à repérer les serviteurs proposés par Bob."""
        if tag.startswith("BACON_SUBSET_"):
            self.subset_tags_seen.add(ent.id)  # pour apprendre les neutres liés à une race
        if tag in BONUS_TAGS:
            me = self.local_player_id
            mine = ent.get("CONTROLLER") == me or (me in self.players and ent.id == self.players[me]["entity_id"])
            if mine and me is not None:
                self.bonus_tags[tag] = ent.get(tag, 0)
        if ent.id == self.game_entity_id and tag in ("TURN", "STATE"):
            if tag == "TURN":
                self._on_turn()
            elif self._fight is not None and ent.get("STATE") == "COMPLETE":
                self._end_fight()          # dernier combat : la partie s'arrête avant le tour suivant
            return
        if tag == "ZONE" and self._fight is not None and ent.get("ZONE") == "PLAY":
            self._fight["entered"].append(ent.id)   # serviteur posé pendant ce combat
        if tag not in ("ZONE", "CONTROLLER", "CARDTYPE") or ent.id in self._seen_in_shop:
            return
        bob = self.bob_player_id
        if bob is None or not ent.is_minion:
            return
        if ent.get("ZONE") == "PLAY" and ent.get("CONTROLLER") == bob and not self.in_combat:
            card = ent.card
            # Seuls les serviteurs du "pool" disent quelles races sont en jeu.
            if card and card.is_pool_minion:
                self._seen_in_shop.add(ent.id)
                races = tuple(r for r in card.races if r != "ALL")  # Amalgame = toutes : ne dit rien
                if len(races) == 1:
                    self.shop_race_counts[races[0]] += 1  # race simple : preuve certaine
                elif races:
                    # Double race (ex. Démon/Dragon) : une SEULE des deux suffit pour qu'il soit
                    # dans la boutique. On garde la paire pour décider plus tard.
                    self._dual_race_seen.append(races)

    # ------------------------------------------------------------- combats
    # Pendant un combat, Bob "joue" le plateau de mon adversaire : ce sont des copies créées au début
    # du combat. On note ces copies (ids entrés en jeu pendant le combat), puis on fige le plateau au
    # premier bloc ATTACK, avant les morts et les invocations des râles d'agonie.
    def _on_turn(self):
        if self.in_combat:
            self._start_fight()
        elif self._fight is not None:
            self._end_fight()

    def _start_fight(self):
        me = self._player_entity(self.local_player_id)
        opp = me.get("NEXT_OPPONENT_PLAYER_ID") if me else None
        self._fight = {"turn": self.turn, "opponent": opp, "entered": [], "board": None,
                       "my_hp": self.my_health, "opp_hp": self.player_health(opp)}

    def on_block(self, block_type, ent=None):
        """Appelé par le lecteur à chaque BLOCK_START (ex. "ATTACK") ; ent = l'entité du bloc."""
        if block_type == "ATTACK" and self._fight is not None and self._fight["board"] is None:
            self._freeze_fight(ent)

    def _freeze_fight(self, attacker=None):
        """Photo des 2 plateaux au premier coup : pour la liste du Lobby et pour le simulateur (combat.py).
        Les effets de Début de combat sont déjà joués à ce moment (leurs bonus sont dans les stats)."""
        bob = self.bob_player_id
        seen, board = set(), []
        for eid in self._fight["entered"]:
            e = self.entities.get(eid)
            if (eid in seen or e is None or not e.is_minion or e.get("CONTROLLER") != bob
                    or e.get("ZONE") != "PLAY"):
                continue
            seen.add(eid)
            board.append(e)
        board.sort(key=lambda e: e.get("ZONE_POSITION", 0))
        f = self._fight
        f["board"] = [snapshot_minion(e) for e in board]
        f["my_board"] = [snapshot_minion(e) for e in self.my_board]
        f["my_tier"] = self.tavern_tier or 1
        opp_hero = self.hero_of(f["opponent"])
        f["opp_tier"] = (opp_hero.get("PLAYER_TECH_LEVEL") if opp_hero else None) or 1
        # Qui attaque en premier : le contrôleur du premier attaquant (moi ou Bob qui joue l'adversaire)
        f["first"] = None if attacker is None else 0 if attacker.get("CONTROLLER") == self.local_player_id else 1
        f["counters"] = (self.player_counters(self.local_player_id), self.player_counters(bob))
        f["deity_stats"] = (self.deity_seen.get(self.my_player_id), self.deity_seen.get(f["opponent"]))
        bonuses = self.my_bonuses
        f["gem"], f["spell"] = bonuses["gem"], bonuses["spell"]
        ge = self.game_entity
        f["cap"] = ge.get("BACON_COMBAT_DAMAGE_CAP") if ge and ge.get("BACON_COMBAT_DAMAGE_CAP_ENABLED") else None

    def _end_fight(self):
        f = self._fight
        self._fight = None
        if f["board"] is None:
            f["board"] = []                  # personne n'a attaqué : plateau vide (ou inconnu)
        # Déités réveillées pendant ce combat : on retient leurs stats pour les prochaines simulations
        for eid in f["entered"]:
            e = self.entities.get(eid)
            if e is not None and e.card_id.startswith("BGFYM_") and e.is_minion:
                owner = self.my_player_id if e.get("CONTROLLER") == self.local_player_id else f["opponent"]
                self.deity_seen[owner] = (e.get("ATK", 1), e.get("HEALTH", 1))
        del f["entered"]
        my_loss = (f["my_hp"] or 0) - (self.my_health or 0)
        opp_loss = (f["opp_hp"] or 0) - (self.player_health(f["opponent"]) or 0)
        f["result"] = "L" if my_loss > 0 else "W" if opp_loss > 0 else "T"
        f["damage"] = my_loss if my_loss > 0 else opp_loss
        self.fights.append(f)

    # Compteurs "de la partie" que le jeu garde sur des enchantements cachés du joueur (comme HDT les lit
    # pour BobsBuddy). En combat, ceux de l'adversaire sont recopiés sur le joueur Bob.
    COUNTER_ENCHANTS = {"BG31_808pe": "beetle", "BG25_008pe": "knights", "BG25_011pe": "undead"}

    def player_counters(self, pid):
        """Bonus des Beetles (+atk, +pv), Eternal Knights morts, +attaque des Morts-vivants de ce joueur."""
        p = self.players.get(pid)
        found = {}
        if p is None:
            return found
        for e in self.entities.values():
            key = self.COUNTER_ENCHANTS.get(e.card_id)
            if key and e.get("ZONE") == "PLAY" and e.get("ATTACHED") == p["entity_id"]:
                a, h = e.get("TAG_SCRIPT_DATA_NUM_1", 0), e.get("TAG_SCRIPT_DATA_NUM_2", 0)
                found[key] = (a, h) if key == "beetle" else a
        return found

    @property
    def current_fight(self):
        """Le combat en cours, une fois les plateaux figés (sinon None)."""
        f = self._fight
        return f if f is not None and f["board"] is not None else None

    def last_fight_against(self, player_id):
        for f in reversed(self.fights):
            if f["opponent"] == player_id:
                return f
        return None

    # ------------------------------------------------------------ joueurs
    @property
    def is_battlegrounds(self):
        return bool(self.game_type and self.game_type.startswith("GT_BATTLEGROUNDS"))

    @property
    def is_duos(self):
        """Partie Duos (GT_BATTLEGROUNDS_DUO...) : le pool contient aussi les cartes Duos."""
        return bool(self.game_type and "DUO" in self.game_type)

    @property
    def local_player_id(self):
        """Moi = le joueur qui a un vrai compte (GameAccountId hi != 0)."""
        for pid, p in self.players.items():
            if p["is_real_account"]:
                return pid
        return None

    @property
    def bob_player_id(self):
        for pid, p in self.players.items():
            if not p["is_real_account"]:
                return pid
        return None

    @property
    def game_entity(self):
        return self.entities.get(self.game_entity_id)

    def _player_entity(self, pid):
        p = self.players.get(pid)
        return self.entities.get(p["entity_id"]) if p else None

    # ------------------------------------------------------ état Battlegrounds
    @property
    def turn(self):
        """Tour BG : le compteur du jeu avance de 2 par tour (recrutement + combat)."""
        t = self.game_entity.get("TURN", 0) if self.game_entity else 0
        return (t + 1) // 2

    @property
    def in_combat(self):
        """Pendant le combat, les serviteurs de Bob en jeu sont ceux de l'adversaire, pas la boutique."""
        ge = self.game_entity
        if ge is None:
            return False
        # Le tour pair du compteur = phase de combat (tour impair = recrutement).
        return ge.get("TURN", 0) % 2 == 0 and ge.get("TURN", 0) > 0

    @property
    def my_hero(self):
        me = self._player_entity(self.local_player_id)
        if me is None:
            return None
        hero = self.entities.get(me.get("HERO_ENTITY"))
        if hero is None or "HERO_PH" in hero.card_id:  # héros provisoire avant le choix
            return None
        return hero

    @property
    def hero_options(self):
        """Héros proposés au choix (avant de choisir) : mes héros "draftables" encore en main.
        Une relance change la carte de la même entité, la liste reste donc à jour."""
        me = self.local_player_id
        player = self._player_entity(me)
        # MULLIGAN_STATE passe à DEALING dès que j'ai cliqué : le choix est fait.
        if (self.my_hero is not None or player is None
                or player.get("MULLIGAN_STATE") in ("DEALING", "WAITING", "DONE")):
            return []
        found = [e for e in self.entities.values()
                 if e.get("CARDTYPE") == "HERO" and e.get("BACON_HERO_CAN_BE_DRAFTED")
                 and e.get("CONTROLLER") == me and e.get("ZONE") == "HAND"]
        return sorted(found, key=lambda e: e.get("ZONE_POSITION", 0))

    @property
    def tavern_tier(self):
        h = self.my_hero
        return h.get("PLAYER_TECH_LEVEL", 1) if h else None

    @property
    def my_health(self):
        h = self.my_hero
        if h is None:
            return None
        return h.get("HEALTH", 0) - h.get("DAMAGE", 0) + h.get("ARMOR", 0)

    @property
    def my_gold(self):
        me = self._player_entity(self.local_player_id)
        if me is None:
            return None
        return me.get("RESOURCES", 0) - me.get("RESOURCES_USED", 0) + me.get("TEMP_RESOURCES", 0)

    def _minions(self, controller, zone):
        found = [e for e in self.entities.values()
                 if e.is_minion and e.get("CONTROLLER") == controller and e.get("ZONE") == zone]
        return sorted(found, key=lambda e: e.get("ZONE_POSITION", 0))

    @property
    def my_board(self):
        return self._minions(self.local_player_id, "PLAY")

    @property
    def my_hand(self):
        return [e for e in self.entities.values()
                if e.get("CONTROLLER") == self.local_player_id and e.get("ZONE") == "HAND"
                and e.get("CARDTYPE") in ("MINION", "BATTLEGROUND_SPELL")]

    @property
    def shop(self):
        return [] if self.in_combat else self._minions(self.bob_player_id, "PLAY")

    @property
    def available_races(self):
        """Races sûres de la partie : vues sur un serviteur à race simple dans la boutique de Bob.
        Si une paire double-race n'est expliquée par aucune race sûre, et qu'il ne manque
        qu'une race, les deux candidates restent dans possible_races (pas de devinette)."""
        return [race for race, _ in self.shop_race_counts.most_common(LOBBY_RACE_COUNT)]

    @property
    def possible_races(self):
        """Races probables mais pas encore prouvées (venant des serviteurs à double race)."""
        sure = set(self.shop_race_counts)
        if len(sure) >= LOBBY_RACE_COUNT:
            return []
        return sorted({r for pair in self._dual_race_seen if not sure & set(pair) for r in pair})

    @property
    def races_complete(self):
        return len(self.shop_race_counts) >= LOBBY_RACE_COUNT

    @property
    def lobby_heroes(self):
        """Les 8 héros du lobby : entités HERO avec une place au classement."""
        heroes = [e for e in self.entities.values()
                  if e.get("CARDTYPE") == "HERO" and e.get("PLAYER_LEADERBOARD_PLACE")
                  and "HERO_PH" not in e.card_id]
        # Un même joueur peut avoir plusieurs entités héros (copie de combat, héros transformé
        # comme Aranna) : on garde celle en jeu (ZONE=PLAY), sinon la plus récente (id le plus grand).
        by_player = {}
        for h in sorted(heroes, key=lambda e: (e.get("ZONE") == "PLAY", e.id)):
            by_player[h.get("PLAYER_ID", h.id)] = h
        # Mon héros : toujours celui désigné par HERO_ENTITY (les copies de combat périment).
        if self.my_hero is not None:
            by_player[self.my_hero.get("PLAYER_ID", self.my_hero.id)] = self.my_hero
        return sorted(by_player.values(), key=lambda e: e.get("PLAYER_LEADERBOARD_PLACE"))

    def hero_of(self, player_id):
        """Héros (entité) du joueur du lobby portant ce PLAYER_ID, ou None."""
        for h in self.lobby_heroes:
            if h.get("PLAYER_ID") == player_id:
                return h
        return None

    def player_health(self, player_id):
        h = self.hero_of(player_id)
        if h is None:
            return None
        return h.get("HEALTH", 0) - h.get("DAMAGE", 0) + h.get("ARMOR", 0)

    @property
    def my_player_id(self):
        """Mon PLAYER_ID dans le lobby (celui que visent NEXT_OPPONENT_PLAYER_ID et les combats)."""
        h = self.my_hero
        return h.get("PLAYER_ID") if h else None

    @property
    def next_opponent_id(self):
        me = self._player_entity(self.local_player_id)
        return me.get("NEXT_OPPONENT_PLAYER_ID") if me else None

    @property
    def deity(self):
        """Déité de la partie (ex. "C'Thun"), comme HDT : tag BACON_GLOBAL_OLD_GOD_DBID de la partie."""
        ge = self.game_entity
        dbf = ge.get("BACON_GLOBAL_OLD_GOD_DBID") if ge else None
        card = get_db().by_dbf.get(dbf) if dbf else None
        return card.name if card else None

    @property
    def is_over(self):
        ge = self.game_entity
        return bool(ge and ge.get("STATE") == "COMPLETE")

    @property
    def my_hero_powers(self):
        """Mes pouvoirs héroïques en jeu (un héros peut en changer ou en avoir deux en cours de partie)."""
        me = self.local_player_id
        found = [e for e in self.entities.values()
                 if e.get("CARDTYPE") == "HERO_POWER" and e.get("CONTROLLER") == me and e.get("ZONE") == "PLAY"
                 and e.card_id]
        return sorted(found, key=lambda e: (e.get("ADDITIONAL_HERO_POWER_INDEX", 0), e.id))

    @property
    def my_bonuses(self):
        """Mes bonus permanents (compteurs de HDT) :
        - spell : +attaque/+vie en plus donnés par mes sorts de taverne (TAVERN_SPELL_*_INCREASE) ;
        - gem   : ce que donne une Gemme de sang (+1/+1 de base, + les tags BACON_BLOODGEMBUFF*VALUE)."""
        t = self.bonus_tags      # comme HDT : la dernière valeur vue sur une entité à moi (joueur, héros...)
        return {"spell": (t.get("TAVERN_SPELL_ATTACK_INCREASE", 0), t.get("TAVERN_SPELL_HEALTH_INCREASE", 0)),
                "gem": (1 + t.get("BACON_BLOODGEMBUFFATKVALUE", 0), 1 + t.get("BACON_BLOODGEMBUFFHEALTHVALUE", 0))}

    def _my_button(self, prefix):
        """Bouton de taverne à moi en jeu (le jeu en recrée un à chaque tour : le plus récent)."""
        me = self.local_player_id
        found = [e for e in self.entities.values()
                 if e.card_id.startswith(prefix) and e.get("CONTROLLER") == me and e.get("ZONE") == "PLAY"]
        return max(found, key=lambda e: e.id) if found else None

    @property
    def upgrade_cost(self):
        """Prix pour monter de tier maintenant (bouton TB_BaconShopTechUp0X : baisse de 1 par tour), None au max."""
        b = self._my_button("TB_BaconShopTechUp")
        return b.get("COST", 0) if b is not None else None

    @property
    def reroll_cost(self):
        """Prix d'une relance de la boutique (1 d'habitude, 0 avec certains pouvoirs ou effets)."""
        b = self._my_button("TB_BaconShop_8p_Reroll_Button")
        return b.get("COST", 1) if b is not None else 1

    @property
    def trinket_offer(self):
        """Trinkets proposés en ce moment (tours 6 et 9) : [(entité, prix réel)]. L'offre = mes Trinkets en SETASIDE
        créés par mon bouton "Lesser / Greater Trinket" (ou un pouvoir) ; le choisi devient ce bouton (CHANGE_ENTITY),
        les autres passent en REMOVEDFROMGAME. COST sur l'entité = prix réduit d'un Trinket "pivot"."""
        if self.in_combat:
            return []
        me = self.local_player_id
        found = []
        for e in self.entities.values():
            if (e.get("CARDTYPE") != "BATTLEGROUND_TRINKET" or e.get("CONTROLLER") != me
                    or e.get("ZONE") != "SETASIDE" or not e.get("BACON_TRINKET") or not e.card_id):
                continue
            creator = self.entities.get(e.get("CREATOR"))
            if creator is None or creator.get("CONTROLLER") != me or creator.get("ZONE") != "PLAY":
                continue                       # copies vues en combat, Trinkets des autres...
            card = get_db().get(e.card_id)
            found.append((e, e.get("COST", card.cost if card else 0)))
        return sorted(found, key=lambda x: x[0].id)

    @property
    def my_trinkets(self):
        """Mes Trinkets achetés (les boutons "Lesser / Greater Trinket" pas encore utilisés ne comptent pas)."""
        me = self.local_player_id
        return [e for e in self.entities.values()
                if e.get("CARDTYPE") == "BATTLEGROUND_TRINKET" and e.get("CONTROLLER") == me
                and e.get("ZONE") == "PLAY" and e.card_id and not e.card_id.startswith("BG30_Trinket_")]

    @property
    def my_placement(self):
        h = self.my_hero
        return h.get("PLAYER_LEADERBOARD_PLACE") if h else None

    def summary(self):
        """Résumé lisible de l'état (pour la console et les tests)."""
        hero = self.my_hero
        return {
            "turn": self.turn,
            "hero": hero.name if hero else None,
            "tier": self.tavern_tier,
            "health": self.my_health,
            "gold": self.my_gold,
            "board": [e.name + (" (G)" if e.is_golden else "") for e in self.my_board],
            "hand": [e.name for e in self.my_hand],
            "shop": [e.name for e in self.shop],
            "races": self.available_races,
            "races_complete": self.races_complete,
            "possible_races": self.possible_races,
            "deity": self.deity,
            "placement": self.my_placement,
            "over": self.is_over,
        }
