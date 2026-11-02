"""Simulateur de combat Battlegrounds (comme BobsBuddy dans HDT) : mes chances de gagner le combat.

Qui fait quoi :
- game.py prend une photo des 2 plateaux au premier coup du combat (bloc ATTACK du log) : à ce moment,
  les effets de "Début de combat" sont déjà joués et leurs bonus sont dans les stats lues.
  Chaque serviteur = un dict (snapshot_minion) : carte, attaque, vie, mots-clés, tier...
- simulate() rejoue ce combat des milliers de fois avec le hasard du jeu (cibles, effets aléatoires,
  qui commence quand les 2 plateaux ont autant de serviteurs) et compte victoires / égalités / défaites,
  les dégâts moyens et les chances de mourir (lethal).
- Règles : celui qui a le plus de serviteurs attaque d'abord ; chacun attaque à tour de rôle avec son
  serviteur suivant (de gauche à droite) ; cible au hasard parmi les Provocations ; dégâts = tier du
  héros gagnant + tiers des serviteurs restants. Mots-clés : Provocation, Bouclier divin, Furie des vents
  (et Méga-), Venimeux, Toxique, Réincarnation, Camouflage, dégâts aux voisins.
- Les effets de cartes connus sont dans EFFECTS (râles d'agonie, Rally, Avenge, "quand un allié
  attaque"...). Les cartes de plateau dont l'effet compte en combat mais n'est pas codé sont listées
  dans result["unknown"] : le pourcentage est alors moins fiable (comme l'avertissement de BobsBuddy).
Écrit à partir des règles du jeu, sans code de HDT ni de BobsBuddy.
"""
import random
import time

from .cards import get_db

MAX_BOARD = 7
MAX_ATTACKS = 400            # sécurité : un combat bloqué (que des 0 d'attaque) finit en égalité
DEFAULT_RUNS = 4000
DEFAULT_SECONDS = 1.5

# Tags du log -> mots-clés du simulateur
KEYWORD_TAGS = {"taunt": "TAUNT", "ds": "DIVINE_SHIELD", "venom": "VENOMOUS", "poison": "POISONOUS",
                "reborn": "REBORN", "stealth": "STEALTH", "cleave": "CLEAVE"}
BEETLE = "BG28_603t"


def snapshot_minion(e):
    """Photo d'un serviteur du log (Entity) en dict simple : ce que le simulateur a besoin de savoir."""
    db = get_db()
    card = db.base(e.card_id) if e.card_id else None
    health = e.get("HEALTH", 0)
    m = {"card_id": e.card_id, "attack": e.get("ATK", 0), "health": health - e.get("DAMAGE", 0),
         "max_health": health, "golden": e.is_golden or e.card_id.endswith("_G"),
         "tier": e.get("TECH_LEVEL") or (card.tier if card else 1) or 1,
         "races": list(card.races) if card else ([e.get("CARDRACE")] if e.get("CARDRACE") else []),
         "windfury": 3 if e.get("MEGA_WINDFURY") or e.get("WINDFURY", 0) >= 3 else 1 if e.get("WINDFURY") else 0,
         "data": (e.get("TAG_SCRIPT_DATA_NUM_1", 0), e.get("TAG_SCRIPT_DATA_NUM_2", 0))}
    for key, tag in KEYWORD_TAGS.items():
        m[key] = bool(e.get(tag))
    return m


# ---------------------------------------------------------------------------------------- serviteurs
class Minion:
    __slots__ = ("base", "card_id", "atk", "hp", "max_hp", "golden", "tier", "races", "taunt", "ds",
                 "venom", "poison", "reborn", "stealth", "cleave", "windfury", "data", "side", "dead",
                 "killer", "flag")

    def __init__(self, d, side):
        card = get_db().base(d["card_id"]) if d.get("card_id") else None
        self.base = card.id if card else d.get("card_id", "")
        self.card_id = d.get("card_id", "")
        self.atk, self.hp = d.get("attack", 0), d.get("health", 1)
        self.max_hp = max(d.get("max_health", self.hp), self.hp)
        self.golden = bool(d.get("golden"))
        self.tier = d.get("tier") or 1
        self.races = set(d.get("races") or [])
        for key in KEYWORD_TAGS:
            setattr(self, key, bool(d.get(key)))
        self.windfury = d.get("windfury", 0)
        self.data = tuple(d.get("data") or (0, 0))
        self.side = side
        self.dead = False
        self.killer = None     # qui l'a tué (pour Leeroy)
        self.flag = False      # déjà déclenché une fois (Scarlet Survivor...)

    def has_race(self, race):
        return race in self.races or "ALL" in self.races

    @property
    def g(self):
        return 2 if self.golden else 1

    def buff(self, atk, hp, fight=None):
        self.atk += atk
        self.hp += hp
        self.max_hp += hp
        if fight is not None and atk:
            fight.on_attack_changed(self)

    def __repr__(self):
        return f"<{self.base} {self.atk}/{self.hp}>"


def token(card_id, side, **extra):
    """Serviteur invoqué en combat, avec les stats de la carte (ou celles données)."""
    card = get_db().get(card_id)
    d = {"card_id": card_id, "attack": card.attack if card else 1, "health": card.health if card else 1,
         "tier": (card.tier if card else 1) or 1, "races": list(card.races) if card else [],
         "golden": card_id.endswith("_G")}
    if card:
        kw = set(card.keywords)
        d.update(taunt="TAUNT" in kw, ds="DIVINE_SHIELD" in kw, reborn="REBORN" in kw,
                 venom="VENOMOUS" in kw, poison="POISONOUS" in kw, windfury=1 if "WINDFURY" in kw else 0)
    d.update(extra)
    if "health" in extra:
        d["max_health"] = extra["health"]
    return Minion(d, side)


# ----------------------------------------------------------------------------------------- un combat
class Fight:
    """Un combat joué une fois. sides[0] = moi, sides[1] = l'adversaire."""

    def __init__(self, mine, theirs, rng, ctx):
        self.rng = rng
        self.ctx = ctx
        self.sides = [[Minion(d, 0) for d in mine], [Minion(d, 1) for d in theirs]]
        self.ptr = [0, 0]                       # prochain attaquant de chaque côté (index)
        self.deaths = [0, 0]                    # morts de ce combat (Rot Hide Gnoll, Avenge...)
        self.avenge = [{}, {}]                  # id(minion) -> morts comptées depuis son dernier Avenge
        self.aberration_deaths = [0, 0]         # Déité : se réveille après 4 Aberrations mortes
        self.dead_aberrations = [[], []]        # Y'Shaarj : les premières Aberrations mortes
        self.deity_done = [False, False]
        self.knight_bonus = [0, 0]              # Eternal Knights morts dans ce combat
        self.kodo_left = {}                     # Stalwart Kodo : 3 fois par combat
        self.beetle = [ctx["beetle"][0][:], ctx["beetle"][1][:]]
        self.undead = list(ctx["undead"])          # "Vos Morts-vivants ont +X d'attaque cette partie"
        self.gem = [list(ctx["gem"][0]), list(ctx["gem"][1])]
        self.deity_bonus = [[0, 0], [0, 0]]     # "Give your Deity +2/+1" avant son réveil
        self.lobster = [ctx["lobster"][0][:], ctx["lobster"][1][:]]

    # -------------------------------------------------------------- outils
    def board(self, side):
        return self.sides[side]

    def alive(self, side):
        return [m for m in self.sides[side] if not m.dead]

    def count(self, side, base):
        """Combien de fois l'effet de cette carte agit (dorée = 2 ; Titus pour les râles en dehors)."""
        return sum(m.g for m in self.sides[side] if m.base == base and not m.dead)

    def summon(self, side, minion, pos=None, aura=True):
        b = self.sides[side]
        if len(b) >= MAX_BOARD:
            return None
        if aura and minion.has_race("UNDEAD"):
            minion.atk += self.undead[side]       # le bonus "de la partie" vaut aussi pour les nouveaux
        pos = len(b) if pos is None else max(0, min(pos, len(b)))
        b.insert(pos, minion)
        if pos < self.ptr[side]:
            self.ptr[side] += 1
        self.on_summon(side, minion)
        return minion

    def remove(self, minion):
        b = self.sides[minion.side]
        i = b.index(minion)
        b.pop(i)
        if i < self.ptr[minion.side]:
            self.ptr[minion.side] -= 1
        return i

    def pick(self, side, cond=lambda m: True, exclude=None):
        found = [m for m in self.sides[side] if not m.dead and m is not exclude and cond(m)]
        return self.rng.choice(found) if found else None

    # -------------------------------------------------------------- dégâts
    def damage(self, target, amount, source=None):
        """Inflige des dégâts ; renvoie les dégâts en trop (pour Wildfire). Le Bouclier divin absorbe tout."""
        if amount <= 0 or target.dead:
            return 0
        if target.ds:
            target.ds = False
            self.on_shield_lost(target)
            return 0
        before = target.hp
        target.hp -= amount
        if source is not None:
            if source.venom or source.poison:
                target.hp = min(target.hp, 0)
                source.venom = False          # Venimeux ne sert qu'une fois
            if target.hp <= 0 and target.killer is None:
                target.killer = source
            if source.has_race("DEMON"):
                for h in self.alive(source.side):
                    if h.base == "BG33_155" and h is not source:
                        h.buff(2 * h.g, 2 * h.g, self)
        return max(0, amount - before)

    # -------------------------------------------------------------- un coup
    def attack(self, side, att, target=None, rally=True):
        enemy = 1 - side
        if target is None or target.dead:
            targets = self.alive(enemy)
            taunts = [m for m in targets if m.taunt]
            visible = [m for m in (taunts or targets) if not m.stealth] or (taunts or targets)
            target = self.rng.choice(visible)
        att.stealth = False
        if rally:
            self.before_attack(side, att, target)
        if att.dead or target.dead:
            return target
        eb = self.sides[enemy]
        victims = [target]
        if att.cleave:                         # Blade Collector : touche aussi les voisins
            i = eb.index(target)
            victims = [m for m in eb[max(0, i - 1):i + 2] if not m.dead]
        hit_back = target.atk
        excess = 0
        for v in victims:
            e = self.damage(v, att.atk, att)
            if v is target:
                excess = e
        self.damage(att, hit_back, target)
        self.after_attack(side, att, target, excess)
        return target

    # -------------------------------------------------------------- morts
    def resolve_deaths(self, first):
        """Retire les morts (côté qui attaque d'abord, de gauche à droite) et joue râles et Réincarnations."""
        while True:
            dead = [(s, m) for s in (first, 1 - first) for m in self.sides[s] if m.hp <= 0 and not m.dead]
            if not dead:
                return
            for _, m in dead:
                m.dead = True
            for side, m in dead:
                pos = self.remove(m)
                self.on_death(side, m, pos)

    def run(self, first):
        side = first
        attacks = 0
        while attacks < MAX_ATTACKS:
            if not self.alive(0) or not self.alive(1):
                break
            att = self.next_attacker(side)
            if att is None:
                if self.next_attacker(1 - side) is None:
                    break                        # plus personne ne peut attaquer : égalité
                side = 1 - side
                continue
            swings = 4 if att.windfury >= 3 else 2 if att.windfury else 1
            for _ in range(swings):
                if att.dead or not self.alive(1 - side) or att.atk <= 0:
                    break
                b = self.sides[side]
                self.ptr[side] = b.index(att) + 1
                self.attack(side, att)
                attacks += 1
                self.resolve_deaths(side)
            side = 1 - side
        return self.alive(0), self.alive(1)

    def next_attacker(self, side):
        b = self.sides[side]
        if not b:
            return None
        n = len(b)
        start = self.ptr[side] % n
        for k in range(n):
            m = b[(start + k) % n]
            if m.atk > 0 and not m.dead:
                return m
        return None

    # ================================================================ effets de cartes
    def on_attack_changed(self, m):
        if m.base == "BG35_814" and not m.flag and m.atk >= 6:      # Scarlet Survivor
            m.flag = True
            m.ds = True

    def on_shield_lost(self, m):
        if m.base == "BG34_405":                                     # Relentless Deflector
            m.taunt = False

    def on_summon(self, side, m):
        for o in self.alive(side):
            if o is m:
                continue
            if o.base == "BG26_802" and m.has_race("BEAST"):         # Banana Slamma
                m.atk *= 3 if o.golden else 2
            elif o.base == "BG35_602" and m.has_race("BEAST"):       # Lurking Leviathan
                m.atk += 3 * o.g
            elif o.base == "BG34_322":                               # Stalwart Kodo
                left = self.kodo_left.setdefault(id(o), 3)
                if left > 0:
                    self.kodo_left[id(o)] = left - 1
                    m.buff(o.atk * o.g, o.max_hp * o.g)
        self.on_attack_changed(m)

    def before_attack(self, side, att, target):
        g = att.g
        b = att.base
        # Rally : "quand ce serviteur attaque"
        if b in RALLY:
            for _ in range(g if b not in RALLY_GOLDEN_IN_TEXT else 1):
                RALLY[b](self, side, att, target)
        for o in self.alive(side):
            if o is att:
                continue
            if o.base == "BG29_816" and att.has_race("DRAGON"):       # Roaring Recruiter
                att.buff(3 * o.g, 1 * o.g, self)
            elif o.base == "BG33_430":                                # Prodigious Tusker
                gem = self.gem[side]
                att.buff(gem[0] * o.g, gem[1] * o.g, self)
            if o.base == "BG36_211" and att.has_race("BEAST"):        # Cage Gnawer
                for m in self.alive(side):
                    if m.has_race("BEAST"):
                        m.buff(2 * o.g, 1 * o.g, self)
        if att.base == "BG36_211" and att.has_race("BEAST"):
            for m in self.alive(side):
                if m.has_race("BEAST"):
                    m.buff(2 * att.g, 1 * att.g, self)

    def after_attack(self, side, att, target, excess):
        enemy = 1 - side
        if att.base == "BGS_126" and excess > 0 and target.hp <= 0:   # Wildfire Elemental
            eb = self.sides[enemy]
            if target in eb:
                i = eb.index(target)
                near = [m for m in (eb[i - 1] if i > 0 else None, eb[i + 1] if i + 1 < len(eb) else None)
                        if m is not None and not m.dead]
                if near and not att.golden:
                    near = [self.rng.choice(near)]
                for m in near:
                    self.damage(m, excess, att)
        if att.base == "BG36_102" and not att.dead:                    # De-volition-ist
            enemies = self.alive(enemy)
            if enemies:
                top = max(m.hp for m in enemies)
                self.damage(self.rng.choice([m for m in enemies if m.hp == top]), att.atk * att.g, att)
        for o in self.alive(side):
            if o.base == "BG36_209":                                  # Ravaging Scorpid
                self.beetle[side][0] += 4 * o.g
                self.beetle[side][1] += 4 * o.g
            if o.base == "BG36_208" and att.base in RALLY and o is not att:   # Deathstrider
                dr = next((m for m in self.alive(side) if m.base in DEATHRATTLE), None)
                for _ in range(o.g):
                    if dr is not None:
                        pos = self.sides[side].index(dr) + 1
                        DEATHRATTLE[dr.base](self, side, dr, pos)

    def on_death(self, side, m, pos):
        self.deaths[side] += 1
        # Leeroy : détruit celui qui l'a tué
        if m.base == "BG23_318" and m.killer is not None and not m.killer.dead:
            m.killer.hp = 0
        # Rot Hide Gnoll, Eternal Knight
        for o in self.alive(side):
            if o.base == "BG25_013":
                o.buff(1 * o.g, 0, self)
            if m.base == "BG25_008" and o.base == "BG25_008":
                o.buff(4 * o.g, 2 * o.g, self)
        if m.base == "BG25_008":
            self.knight_bonus[side] += 1
        # Avenge (X) : toutes les X morts alliées
        for o in self.alive(side):
            if o.base in AVENGE:
                n, fn = AVENGE[o.base]
                k = self.avenge[side].get(id(o), 0) + 1
                if k >= n:
                    k = 0
                    for _ in range(o.g):
                        fn(self, side, o)
                self.avenge[side][id(o)] = k
        # Râle d'agonie (Titus Rivendare = une fois de plus, doré = deux fois de plus)
        if m.base in DEATHRATTLE:
            times = 1 + sum(o.g for o in self.alive(side) if o.base == "BG25_354")
            for _ in range(times):
                DEATHRATTLE[m.base](self, side, m, pos)
                for o in self.alive(side):
                    if o.base == "BG35_342":                          # Falling Sky Golem
                        o.buff(4 * o.g, 2 * o.g, self)
        # Réincarnation : revient avec 1 PV, sans ses bonus
        if m.reborn:
            r = token(m.card_id, side, health=1, reborn=False, tier=m.tier)
            if m.base == "BG25_008":                                  # Eternal Knight : garde son aura
                n = self.ctx["knights"][side] + self.knight_bonus[side]
                r.atk = (4 + 4 * n) * m.g
            r.reborn = False
            if self.summon(side, r, pos):
                for o in self.alive(side):
                    if o is r:
                        continue
                    if o.base == "BG36_514":                          # Barrier Banshee
                        o.ds = True
                        o.buff(8 * o.g, 8 * o.g, self)
                    elif o.base == "BG36_515":                        # Snazzy Phantom
                        undead = [u for u in self.alive(side) if u.has_race("UNDEAD")]
                        if undead:
                            undead[-1].buff(r.atk * o.g, r.atk * o.g, self)
        # Déité : après 4 Aberrations alliées mortes, elle se réveille
        if m.has_race("ABERRATION") and not m.base.startswith("BGFYM"):
            self.dead_aberrations[side].append(m)
            self.aberration_deaths[side] += 1
            if self.aberration_deaths[side] >= 4 and not self.deity_done[side] and self.ctx["deity_id"]:
                self.deity_done[side] = True
                self.awaken_deity(side)
        if m.base == "BGFYM_011":                                     # Y'Shaarj
            for a in self.dead_aberrations[side][:2 * m.g]:
                self.summon(side, token(a.card_id, side, attack=a.atk, health=a.max_hp), pos, aura=False)

    def buff_deity(self, side, atk, hp):
        deity = next((m for m in self.alive(side) if m.base.startswith("BGFYM")), None)
        if deity is not None:
            deity.buff(atk, hp, self)
        else:
            self.deity_bonus[side][0] += atk
            self.deity_bonus[side][1] += hp

    def awaken_deity(self, side):
        atk, hp = self.ctx["deity_stats"][side]
        atk, hp = atk + self.deity_bonus[side][0], hp + self.deity_bonus[side][1]
        d = token(self.ctx["deity_id"], side, attack=atk, health=hp)
        if self.summon(side, d, 0) and self.ctx["deity_id"] == "BGFYM_000":   # C'Thun : partage ses stats
            others = [m for m in self.alive(side) if m is not d]
            for _ in range(atk if others else 0):
                self.rng.choice(others).buff(1, 0, self)
            for _ in range(hp if others else 0):
                self.rng.choice(others).buff(0, 1)


# --------------------------------------------------------------------------------- tables d'effets
def _summon_n(card_id, n, golden_id=None):
    def fn(f, side, m, pos):
        cid = golden_id if (golden_id and m.golden) else card_id
        for _ in range(n * (1 if golden_id else m.g)):
            f.summon(side, token(cid, side), pos)
    return fn


def _beetles(n):
    def fn(f, side, m, pos):
        a, h = f.beetle[side]
        for _ in range(n * m.g):
            f.summon(side, token(BEETLE, side, attack=a, health=h, races=["BEAST"]), pos)
    return fn


def _buff_all(atk, hp, race=None):
    def fn(f, side, m, pos):
        for o in f.alive(side):
            if race is None or o.has_race(race):
                o.buff(atk * m.g, hp * m.g, f)
    return fn


def _buff_random(atk, hp, race):
    def fn(f, side, m, pos):
        o = f.pick(side, lambda x: x.has_race(race))
        if o:
            o.buff(atk * m.g, hp * m.g, f)
    return fn


def _tentacles(f, side, m, pos):
    for _ in range(m.g):
        f.summon(side, token("", side, attack=0, health=2, taunt=True, races=["ABERRATION"]), pos)
    for o in f.alive(side):
        o.buff(m.g, 0, f)


def _lobster(f, side, m, pos):
    a, h = f.lobster[side]
    o = f.pick(side, lambda x: x.has_race("BEAST"))
    if o:
        o.buff(a * m.g, h * m.g, f)


def _mummifier(f, side, m, pos):
    for _ in range(m.g):
        o = f.pick(side, lambda x: x.has_race("UNDEAD") and not x.reborn)
        if o:
            o.reborn = True


def _ghastcoiler(f, side, m, pos):
    pool = f.ctx["deathrattle_pool"]
    for _ in range(2 * m.g):
        if pool:
            f.summon(side, token(f.rng.choice(pool), side), pos)


def _skitterer(f, side, m, pos):
    f.beetle[side][0] += 5 * m.g
    f.beetle[side][1] += 5 * m.g
    _beetles(1)(f, side, m, pos)


def _nadina(f, side, m, pos):
    dragons = [o for o in f.alive(side) if o.has_race("DRAGON") and not o.ds]
    f.rng.shuffle(dragons)
    for o in dragons[:3 * m.g]:
        o.ds = True


def _plaguerunner(f, side, m, pos):
    f.undead[side] += 2 * m.g
    _buff_all(2, 0, "UNDEAD")(f, side, m, pos)


def _deity_buff(atk, hp):
    def fn(f, side, m, pos):
        f.buff_deity(side, atk * m.g, hp * m.g)
    return fn


def _knight(f, side, m, pos):
    bonus = f.ctx["knights"][side] + f.knight_bonus[side]
    cid = "BG25_008_G" if m.golden else "BG25_008"
    g = 2 if m.golden else 1
    f.summon(side, token(cid, side, attack=(4 + 4 * bonus) * g, health=(2 + 2 * bonus) * g), pos)


DEATHRATTLE = {
    "BG31_803": _beetles(1),                                 # Buzzing Vermin
    "BG31_801": _beetles(1),                                 # Forest Rover
    "BG36_209": _beetles(1),                                 # Ravaging Scorpid
    "BG31_809": _skitterer,                                  # Turquoise Skitterer
    "BG29_611": _summon_n("BG_BOT_312t", 1, "TB_BaconUps_032t"),   # Cord Puller
    "BG28_300": _summon_n("BG_ICC_026t", 2),                 # Harmless Bonehead
    "BG30_125": _summon_n("BG_ICC_026t", 3),                 # Cadaver Caretaker
    "BG25_010": _summon_n("BG25_010t", 1),                   # Handless Forsaken
    "BG35_604": _summon_n("BG19_010", 2, "BG19_010_G"),      # Sewer Lord
    "BG19_010": _summon_n("BG19_010t", 1, "BG19_010_Gt"),    # Sewer Rat
    "BG36_116": _tentacles,                                  # Underrot Spawn
    "BG25_022": _buff_random(1, 2, "UNDEAD"),                # Scarlet Skull
    "BG36_202": _lobster,                                    # Tasty Lobster
    "BG28_309": _mummifier,                                  # Mummifier
    "BGS_008": _ghastcoiler,                                 # Ghastcoiler
    "BGS_018": _buff_all(7, 7, "BEAST"),                     # Goldrinn
    "BG33_828": _buff_all(6, 6),                             # Ship Master Eudora
    "BG34_690": _plaguerunner,                               # Plaguerunner
    "BGS_040": _nadina,                                      # Nadina the Red
    "BG25_009": _knight,                                     # Eternal Summoner
    "BG36_113": _deity_buff(2, 1),                           # Drifting Sacrifice
    "BG36_318": _deity_buff(2, 1),                           # Faceless Converter (sans son bonus de sorts)
}


def _rally_summon(card_id, golden_id):
    def fn(f, side, m, target):
        cid = golden_id if m.golden else card_id
        f.summon(side, token(cid, side), f.sides[side].index(m) + 1)
    return fn


def _rally_bat(f, side, m, target):
    f.summon(side, token("BG36_200t", side), f.sides[side].index(m) + 1)


def _rally_gain(atk, hp):
    def fn(f, side, m, target):
        m.buff(atk, hp, f)
    return fn


def _rally_wolf(f, side, m, target):
    for o in f.alive(side):
        if o is not m:
            o.buff(4, 1, f)


def _rally_bonker(f, side, m, target):
    gem = f.gem[side]
    for o in f.alive(side):
        if o is not m:
            o.buff(gem[0], gem[1], f)


def _rally_underdog(f, side, m, target):
    m.buff(target.atk, 0, f)


def _rally_sindorei(f, side, m, target):
    target.reborn = False
    target.taunt = False


def _rally_bile(f, side, m, target):
    o = f.pick(side, lambda x: x.has_race("MURLOC") and not x.venom, exclude=m)
    if o:
        o.venom = True


def _rally_brood(f, side, m, target):
    m.ds = True


def _rally_ravager(f, side, m, target):
    eb = f.sides[1 - side]
    i = eb.index(target)
    near = [o for o in (eb[i - 1] if i > 0 else None, eb[i + 1] if i + 1 < len(eb) else None) if o]
    hits = [target] + (near if m.golden else ([f.rng.choice(near)] if near else []))
    for o in hits:
        f.damage(o, m.atk, m)


def _rally_gems(n):
    def fn(f, side, m, target):
        gem = f.gem[side]
        m.buff(gem[0] * n, gem[1] * n, f)
    return fn


def _rally_refiner(f, side, m, target):
    f.gem[side][0] += 1
    f.gem[side][1] += 2


def _rally_dragonbreath(f, side, m, target):
    sp = f.ctx["spell"][side]
    a, h = 3 + sp[0], 2 + sp[1]
    for o in f.alive(side):
        times = 1 + o.has_race("DRAGON") + o.ds
        o.buff(a * times, h * times, f)


def _rally_last_one(f, side, m, target):
    races = {r for o in f.alive(side) for r in o.races if r != "ALL"}
    for race in races:
        o = f.pick(side, lambda x: x.has_race(race))
        if o:
            o.buff(15 * m.g, 15 * m.g, f)


def _rally_jailbird(f, side, m, target):
    k = 2 if m.golden else 1
    golem = f.summon(side, token("", side, attack=m.atk * k, health=m.hp * k), f.sides[side].index(m) + 1)
    if golem and not target.dead:
        f.attack(side, golem, target, rally=False)
        f.resolve_deaths(side)


RALLY = {
    "BG36_200": _rally_bat,                                  # Flittering Bat
    "BG29_888": _rally_gain(2, 0),                           # Glim Guardian
    "BG33_886": _rally_gems(1),                              # Tusked Camper
    "BG36_207": _rally_wolf,                                 # Wolf Pup
    "BG20_104": _rally_bonker,                               # Bonker
    "BG34_604": _rally_underdog,                             # Heroic Underdog
    "BG25_016": _rally_sindorei,                             # Sin'dorei Straight Shot
    "BG33_318": _rally_bile,                                 # Bile Spitter
    "BG36_210": _rally_summon("BG36_202", "BG36_202_G"),     # Hoarding Hyena
    "BG36_849": _rally_brood,                                # Heroic Broodmother
    "BG27_017": _rally_ravager,                              # Obsidian Ravager
    "BG33_883": _rally_gems(4),                              # Razorfen Vineweaver
    "BG33_885": _rally_refiner,                              # Sanguine Refiner
    "BG36_241": _rally_dragonbreath,                         # Crimson Vindicator
    "BG34_320": _rally_last_one,                             # The Last One Standing
    "BG36_333": _rally_jailbird,                             # Jailbird Juggernaut
}
# Rally dont la version dorée est déjà gérée dans la fonction (ne pas la jouer 2 fois)
RALLY_GOLDEN_IN_TEXT = {"BG36_210", "BG27_017", "BG36_333"}


def _avenge_deflector(f, side, m):
    m.ds = True
    m.taunt = True


def _avenge_tycoon(f, side, m):
    bonus = f.ctx["knights"][side] + f.knight_bonus[side]
    k = f.summon(side, token("BG25_008", side, attack=4 + 4 * bonus, health=2 + 2 * bonus),
                 f.sides[side].index(m) + 1)
    if k and f.alive(1 - side):
        f.attack(side, k)


AVENGE = {
    "BG34_405": (3, _avenge_deflector),                      # Relentless Deflector
    "BG34_403": (5, _avenge_tycoon),                         # Eternal Tycoon
}

# Effets codés ailleurs dans Fight (ne pas les signaler comme inconnus)
HANDLED_ELSEWHERE = {"BG35_814", "BG34_405", "BG26_802", "BG35_602", "BG34_322", "BG29_816", "BG33_430",
                     "BG36_211", "BGS_126", "BG36_102", "BG36_209", "BG36_208", "BG23_318", "BG25_013",
                     "BG25_008", "BG25_354", "BG36_514", "BG36_515", "BG33_155", "BG26_817", "BGFYM_011",
                     "BGFYM_000", "BG35_342"}
# Effets "en combat" qui ne changent rien au résultat du combat (main, Taverne, héros après le combat)
NO_COMBAT_EFFECT = {"BG26_174", "BG29_300", "BG36_243", "BG36_362", "BG26_523", "BG23_017", "BG27_016",
                    "BG32_873", "BG36_733"}
# Mots qui, dans le texte d'une carte, veulent dire "ça agit pendant le combat"
_COMBAT_WORDS = ("Deathrattle", "Rally", "Avenge", "attacks", "this combat", "takes damage", "dies",
                 "is Reborn", "summon", "Summon", "loses Divine Shield", "deals damage")
# ... sauf ces effets qui ne changent rien au combat (cartes ajoutées en main, Taverne, partie suivante)
_SHOP_ONLY = ("Get a", "Get an", "Get 2", "Get 3", "Tavern", "Lockbox", "Blood Gems give", "next turn",
              "Refresh", "spells give", "Bounty", "Chromadrake", "Repair Job", "Fodder")


def is_unknown_effect(card):
    """Carte dont l'effet compte en combat mais n'est pas simulé (prévient l'utilisateur)."""
    if card is None or card.id in DEATHRATTLE or card.id in RALLY or card.id in AVENGE \
            or card.id in HANDLED_ELSEWHERE or card.id in NO_COMBAT_EFFECT:
        return False
    text = card.text
    if not any(w in text for w in _COMBAT_WORDS):
        return False
    if "Start of Combat" in text and not any(w in text for w in ("Deathrattle", "Rally", "Avenge", "attacks")):
        return False                     # déjà joué au moment de la photo
    if any(w in text for w in _SHOP_ONLY) and not any(w in text.split("Get")[0] for w in ("Summon", "Give")):
        return False
    return True


# ------------------------------------------------------------------------------------- simulation
def _side_ctx(board, counters):
    """Bonus "de la partie" d'un côté : stats des Beetles, des Tasty Lobsters... Les compteurs du log
    (enchantements cachés du joueur, voir game.player_counters) d'abord, sinon ce que montrent les cartes."""
    if counters.get("beetle"):
        a, h = counters["beetle"]
        beetle = [2 + a, 2 + h]
    else:
        beetle = [2, 2]
    for m in board:
        base = get_db().base(m["card_id"])
        if base and base.id in ("BG31_803", "BG31_801", "BG31_809", "BG36_209"):
            a, h = m.get("data") or (0, 0)
            if a >= 2 and h >= 2 and not counters.get("beetle"):
                beetle = [max(beetle[0], a), max(beetle[1], h)]
    lobster = [2, 1]
    for m in board:
        base = get_db().base(m["card_id"])
        if base and base.id == "BG36_202":
            a, h = m.get("data") or (0, 0)
            if a > 0:
                lobster = [a, h]
    return beetle, lobster


def _deathrattle_pool():
    db = get_db()
    return [c.id for c in db.pool_minions() if c.id in DEATHRATTLE and c.id != "BGS_008"]


def simulate(mine, theirs, my_tier=1, their_tier=1, my_health=None, their_health=None, first=None,
             runs=DEFAULT_RUNS, seconds=DEFAULT_SECONDS, seed=None, deity=None, deity_stats=None,
             counters=({}, {}), gem=((1, 1), (1, 1)), spell=((0, 0), (0, 0)), cap=None):
    """Rejoue le combat 'runs' fois (ou pendant 'seconds' au plus) et renvoie les pourcentages.

    mine / theirs : listes de dicts (snapshot_minion). first : 0 = j'attaque en premier, 1 = lui,
    None = la règle du jeu (plus de serviteurs, sinon au hasard). deity = id de carte de la Déité
    (BGFYM_000 C'Thun, BGFYM_011 Y'Shaarj) et deity_stats = ((atk, pv) pour moi, pour lui).
    counters = compteurs de la partie de chaque côté (beetle, knights, undead : voir game.player_counters).
    gem / spell = bonus des Gemmes de sang et des sorts de taverne de chaque côté ; cap = plafond de
    dégâts du combat (tag BACON_COMBAT_DAMAGE_CAP de la partie, qui monte avec les tours)."""
    rng = random.Random(seed)
    counters = [c or {} for c in counters]
    b0, l0 = _side_ctx(mine, counters[0])
    b1, l1 = _side_ctx(theirs, counters[1])
    knights = tuple(c.get("knights", 0) for c in counters)
    ctx = {"beetle": (b0, b1), "lobster": (l0, l1), "deathrattle_pool": _deathrattle_pool(),
           "deity_id": deity, "deity_stats": deity_stats or ((1, 1), (1, 1)), "knights": knights,
           "undead": tuple(c.get("undead", 0) for c in counters),
           "gem": gem, "spell": spell}
    win = tie = loss = 0
    dealt = taken = 0
    my_lethal = their_lethal = 0
    start = time.perf_counter()
    n = 0
    while n < runs:
        if n % 200 == 0 and n and time.perf_counter() - start > seconds:
            break
        n += 1
        f = Fight(mine, theirs, rng, ctx)
        who = first
        if who is None:
            a, b = len(mine), len(theirs)
            who = 0 if a > b else 1 if b > a else rng.randint(0, 1)
        left0, left1 = f.run(who)
        if left0 and not left1:
            win += 1
            d = my_tier + sum(m.tier for m in left0)
            d = min(d, cap) if cap else d
            dealt += d
            if their_health is not None and d >= their_health:
                their_lethal += 1
        elif left1 and not left0:
            loss += 1
            d = their_tier + sum(m.tier for m in left1)
            d = min(d, cap) if cap else d
            taken += d
            if my_health is not None and d >= my_health:
                my_lethal += 1
        else:
            tie += 1
    n = max(n, 1)
    db = get_db()
    unknown = sorted({c.name for c in (db.base(m["card_id"]) for m in list(mine) + list(theirs))
                      if is_unknown_effect(c)})
    return {
        "runs": n, "win": 100 * win / n, "tie": 100 * tie / n, "loss": 100 * loss / n,
        "dealt": dealt / win if win else 0, "taken": taken / loss if loss else 0,
        "avg_dealt": dealt / n, "avg_taken": taken / n,
        "lethal": 100 * their_lethal / n, "lethal_me": 100 * my_lethal / n,
        "unknown": unknown, "first": first,
    }


# ------------------------------------------------------------------------- pour l'affichage (HUD)
DEITY_IDS = {"C'Thun": "BGFYM_000", "Y'Shaarj": "BGFYM_011"}
LIVE_RUNS, LIVE_SECONDS = 6000, 1.5          # combat en cours : comme BobsBuddy (10 000 en 1,5 s max)
PAST_RUNS, PAST_SECONDS = 1500, 0.3          # anciens combats de la partie (moyenne de la partie)
PREVIEW_RUNS, PREVIEW_SECONDS = 1500, 0.4    # estimation pendant le recrutement


def _odds(game, mine, theirs, my_tier, opp_tier, opp, first, cap, deity_stats, gem, spell, runs, seconds,
          counters=({}, {})):
    return simulate(mine, theirs, my_tier, opp_tier, my_health=game.my_health,
                    their_health=game.player_health(opp), first=first, runs=runs, seconds=seconds,
                    deity=DEITY_IDS.get(game.deity), deity_stats=tuple(s or (1, 1) for s in deity_stats),
                    gem=(gem, (1, 1)), spell=(spell, (0, 0)), cap=cap, counters=counters)


def fight_odds(game, f, runs=LIVE_RUNS, seconds=LIVE_SECONDS):
    """Chances d'un combat figé par game.py (calculées une seule fois, gardées dans f["sim"])."""
    if f.get("sim") is None and "my_board" in f:
        f["sim"] = _odds(game, f["my_board"], f["board"], f["my_tier"], f["opp_tier"], f["opponent"],
                         f["first"], f["cap"], f["deity_stats"], f["gem"], f["spell"], runs, seconds,
                         f.get("counters", ({}, {})))
    return f.get("sim")


def _hero_name(game, pid):
    h = game.hero_of(pid)
    return h.name if h else None


def combat_info(game):
    """Ce que le panneau montre : le combat en cours (ou une estimation avant le combat) et la moyenne
    de mes chances sur les combats de la partie. Appelé dans le fil qui lit le log, pas dans l'écran."""
    if game is None or not game.is_battlegrounds or game.is_duos:
        return None
    for f in game.fights:                      # anciens combats : vite, pour la moyenne de la partie
        fight_odds(game, f, PAST_RUNS, PAST_SECONDS)
    done = [f for f in game.fights if f.get("sim")]
    info = {"games_fights": [{"turn": f["turn"], "win": f["sim"]["win"], "tie": f["sim"]["tie"],
                              "loss": f["sim"]["loss"], "result": f["result"]} for f in done]}
    if done:
        info["avg_win"] = sum(f["sim"]["win"] for f in done) / len(done)
        info["avg_tie"] = sum(f["sim"]["tie"] for f in done) / len(done)
        info["avg_loss"] = sum(f["sim"]["loss"] for f in done) / len(done)
        info["expected_wins"] = sum(f["sim"]["win"] for f in done) / 100
        info["actual_wins"] = sum(f["result"] == "W" for f in done)
    cur = game.current_fight
    if cur is not None:
        info["now"] = dict(fight_odds(game, cur), live=True, turn=cur["turn"],
                           opponent=_hero_name(game, cur["opponent"]))
        return info
    if game.is_over:
        return info
    # Recrutement : mon plateau actuel contre le plateau ATTENDU de mon prochain adversaire : son dernier
    # plateau vu, "vieilli" jusqu'à ce tour (planner.expected_board), ou un plateau typique s'il est inconnu.
    from .planner import expected_board          # (ici : planner importe combat)
    opp = game.next_opponent_id
    if opp is None:
        return info
    last = game.last_fight_against(opp)
    if last is not None and "my_board" not in last:
        last = None
    mine = [snapshot_minion(e) for e in game.my_board]
    if not mine:
        return info
    opp_hero = game.hero_of(opp)
    opp_tier = (opp_hero.get("PLAYER_TECH_LEVEL") if opp_hero else None) or (last["opp_tier"] if last else 1)
    ge = game.game_entity
    cap = ge.get("BACON_COMBAT_DAMAGE_CAP") if ge and ge.get("BACON_COMBAT_DAMAGE_CAP_ENABLED") else None
    key = (repr(mine), last["turn"] if last else None, opp, opp_tier, game.turn, cap)
    cache = getattr(game, "_combat_preview", None)
    if cache is None or cache[0] != key:
        theirs, basis = expected_board(game.turn, opp_tier, last)
        bonuses = game.my_bonuses
        sim = _odds(game, mine, theirs, game.tavern_tier or 1, opp_tier, opp, None,
                    cap, (game.deity_seen.get(game.my_player_id), game.deity_seen.get(opp)),
                    bonuses["gem"], bonuses["spell"], PREVIEW_RUNS, PREVIEW_SECONDS,
                    (game.player_counters(game.local_player_id), (last or {}).get("counters", ({}, {}))[1]))
        cache = (key, dict(sim, basis=basis))
        game._combat_preview = cache
    info["now"] = dict(cache[1], live=False, turn=game.turn, seen_turn=last["turn"] if last else None,
                       opponent=_hero_name(game, opp), cap=cap)
    return info
