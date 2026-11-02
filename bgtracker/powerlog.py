"""Lecteur de Power.log : transforme les lignes du journal de Hearthstone en entités + tags.

Qui fait quoi :
- Hearthstone écrit chaque changement de partie dans Power.log (une ligne = un événement).
- PowerLogParser lit ces lignes une par une (feed_line) et tient à jour un objet Game.
- Game (game.py) garde toutes les entités (héros, serviteurs, joueurs) et leurs tags.
On ne lit que les lignes "GameState.DebugPrintPower" : les lignes "PowerTaskList" répètent
les mêmes événements (un peu plus tard, pour l'animation) et créeraient des doublons.
"""
import re  # expressions régulières : reconnaître la forme de chaque ligne

from .game import Game  # l'état de la partie que l'on remplit

# Préfixe des lignes utiles. Exemple :
# "D 12:00:41.4016168 GameState.DebugPrintPower() -     TAG_CHANGE Entity=10 tag=TURN value=1"
_PREFIX = re.compile(r"^D ([\d:.]+) GameState\.DebugPrint(Power|Game)\(\) - (.*)$")

# Une ligne de tag à l'intérieur d'un bloc (FULL_ENTITY, SHOW_ENTITY...) : "tag=ZONE value=PLAY"
_TAG = re.compile(r"^tag=(\S+) value=(\S*)")
# Création d'une entité : "FULL_ENTITY - Creating ID=31 CardID=TB_BaconShop_HERO_PH"
_FULL_CREATE = re.compile(r"^FULL_ENTITY - Creating ID=(\d+) CardID=(\S*)")
# Variante "FULL_ENTITY - Updating [entityName=... id=31 ...] CardID=..."
_FULL_UPDATE = re.compile(r"^FULL_ENTITY - Updating (.+) CardID=(\S*)")
# Révélation d'une carte cachée : "SHOW_ENTITY - Updating Entity=254 CardID=BG29_813e"
_SHOW = re.compile(r"^SHOW_ENTITY - Updating Entity=(.+) CardID=(\S*)")
# Transformation (ex. héros provisoire -> vrai héros) : même forme que SHOW_ENTITY
_CHANGE = re.compile(r"^CHANGE_ENTITY - Updating Entity=(.+) CardID=(\S*)")
# Entité cachée de nouveau : "HIDE_ENTITY - Entity=[...] tag=ZONE value=SETASIDE"
_HIDE = re.compile(r"^HIDE_ENTITY - Entity=(.+) tag=(\S+) value=(\S*)")
# Changement d'un seul tag : "TAG_CHANGE Entity=Player#1234 tag=TURN value=1 "
_TAG_CHANGE = re.compile(r"^TAG_CHANGE Entity=(.+) tag=(\S+) value=(\S*)")
# Entité de la partie : "GameEntity EntityID=10"
_GAME_ENTITY = re.compile(r"^GameEntity EntityID=(\d+)")
# Joueur : "Player EntityID=11 PlayerID=4 GameAccountId=[hi=144115198130930503 lo=28027830]"
_PLAYER = re.compile(r"^Player EntityID=(\d+) PlayerID=(\d+) GameAccountId=\[hi=(\d+) lo=(\d+)\]")
# En-tête de partie : "PlayerID=4, PlayerName=Player#1234" et "GameType=GT_BATTLEGROUNDS"
_PLAYER_NAME = re.compile(r"^PlayerID=(\d+), PlayerName=(.*)$")
_GAME_TYPE = re.compile(r"^GameType=(\S+)")
# Début de bloc : "BLOCK_START BlockType=ATTACK Entity=[...] ..."
_BLOCK = re.compile(r"^BLOCK_START BlockType=(\S+)(?: Entity=(\[.*?\bid=\d+[^\]]*\]|\S+))?")
# Référence entre crochets : "[entityName=Thorim, Stormlord id=91 zone=PLAY ...]" -> on garde id=91
_BRACKET_ID = re.compile(r"\bid=(\d+)\b")


class PowerLogParser:
    """Lit Power.log ligne par ligne et met à jour self.game (None tant qu'aucune partie)."""

    def __init__(self, on_new_game=None):
        self.game = None                # partie en cours (objet Game)
        self.games = []                 # toutes les parties vues (utile pour rejouer un vieux log)
        self._current = None            # entité dont on lit les lignes "tag=..." du bloc en cours
        self._on_new_game = on_new_game  # fonction appelée à chaque nouvelle partie (optionnel)
        self._time = None               # heure de la dernière ligne lue

    # ------------------------------------------------------------------ entrée
    def feed_line(self, line):
        """Traite UNE ligne brute du fichier. Les lignes inutiles sont ignorées."""
        m = _PREFIX.match(line.rstrip("\r\n"))
        if not m:
            return                      # ligne PowerTaskList, Choices, etc. : on ignore
        self._time = m.group(1)         # heure de la ligne (ex. 12:00:41.4016168)
        kind, body = m.group(2), m.group(3).strip()  # strip() enlève l'indentation
        if kind == "Game":
            self._read_game_header(body)
        else:
            self._read_power(body)

    def feed_lines(self, lines):
        """Pratique pour les tests : traite toute une liste/fichier de lignes."""
        for line in lines:
            self.feed_line(line)

    # ---------------------------------------------------------------- en-tête
    def _read_game_header(self, body):
        if self.game is None:
            return
        if m := _GAME_TYPE.match(body):
            self.game.game_type = m.group(1)          # ex. GT_BATTLEGROUNDS
        elif m := _PLAYER_NAME.match(body):
            # PlayerID -> nom affiché (BattleTag pour moi, nom de Bob pour l'aubergiste)
            self.game.player_names[int(m.group(1))] = m.group(2)

    # ------------------------------------------------------------ événements
    def _read_power(self, body):
        # 1) Ligne "tag=X value=Y" : elle complète l'entité du bloc en cours.
        if m := _TAG.match(body):
            if self._current is not None:
                self._current.set_tag(m.group(1), m.group(2))
            return
        # Toute autre ligne termine le bloc de tags en cours.
        self._current = None

        if body == "CREATE_GAME":
            self._start_game()
            return
        g = self.game
        if g is None:
            return                      # événements avant le début d'une partie : ignorés

        if m := _TAG_CHANGE.match(body):
            ent = self._resolve(m.group(1))
            if ent is not None:
                g.tag_change(ent, m.group(2), m.group(3))
        elif m := _FULL_CREATE.match(body):
            self._current = g.create_entity(int(m.group(1)), m.group(2))
        elif m := _SHOW.match(body) or _CHANGE.match(body) or _FULL_UPDATE.match(body):
            ent = self._resolve(m.group(1))
            if ent is not None:
                if m.group(2):
                    ent.card_id = m.group(2)  # la carte est maintenant connue (ou a changé)
                self._current = ent           # les lignes "tag=" suivantes la complètent
        elif m := _HIDE.match(body):
            ent = self._resolve(m.group(1))
            if ent is not None:
                g.tag_change(ent, m.group(2), m.group(3))
        elif m := _BLOCK.match(body):
            g.on_block(m.group(1), self._resolve(m.group(2)) if m.group(2) else None)
        elif m := _GAME_ENTITY.match(body):
            self._current = g.create_entity(int(m.group(1)), "")
            g.game_entity_id = int(m.group(1))
        elif m := _PLAYER.match(body):
            eid, pid = int(m.group(1)), int(m.group(2))
            self._current = g.create_entity(eid, "")
            # hi=0 : compte fictif = Bob (l'aubergiste). Sinon c'est un vrai compte = moi.
            g.players[pid] = {"entity_id": eid, "is_real_account": m.group(3) != "0"}
        # BLOCK_END / META_DATA : pas utiles pour l'instant.

    def _start_game(self):
        """CREATE_GAME : une nouvelle partie commence, on repart de zéro."""
        self.game = Game()
        self.game.start_time = (self._time or "")[:8]   # "12:00:41" : sert à reconnaître la partie
        self.games.append(self.game)
        if self._on_new_game:
            self._on_new_game(self.game)

    def _resolve(self, ref):
        """Trouve l'entité désignée par 'ref', qui peut prendre 4 formes :
        un numéro ("254"), des crochets ("[entityName=... id=91 ...]"),
        "GameEntity", ou un nom de joueur ("Player#1234")."""
        g = self.game
        ref = ref.strip()
        if ref.isdigit():
            return g.entities.get(int(ref))
        if ref.startswith("["):
            m = _BRACKET_ID.search(ref)
            return g.entities.get(int(m.group(1))) if m else None
        if ref == "GameEntity":
            return g.entities.get(g.game_entity_id)
        return g.entity_by_player_name(ref)
