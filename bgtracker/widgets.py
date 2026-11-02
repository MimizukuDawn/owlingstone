"""Éléments d'affichage façon Hearthstone Deck Tracker, partagés par la liste des tiers (ui.py) et le
panneau de droite (hud.py).

- Tile : une ligne "carte" dessinée à la main : pastille à gauche (tier, place...), nom en blanc ombré,
  l'image de la carte à droite fondue dans le fond, et des badges ronds (attaque/vie, PV, tier...).
  Le contour dit l'état : or = dans la boutique, vert = à moi / à acheter, bleu = dans ma main.
- Thumbs : une rangée de petites images de cartes (aperçu d'une comp fermée).
- CardPreview : l'image entière de la carte, montrée à côté du panneau quand la souris passe sur une tuile.
- Pill / pill_html : petites étiquettes colorées (difficulté, race, placement moyen).
"""
import re

from PySide6.QtCore import QRectF, Qt
from PySide6.QtGui import QColor, QFont, QFontMetrics, QLinearGradient, QPainter, QPainterPath, QPen
from PySide6.QtWidgets import QApplication, QLabel, QSizePolicy, QWidget

BG = QColor("#1c1e25")
TIER_COLORS = {1: "#8a8f99", 2: "#3f9d55", 3: "#3a7bd5", 4: "#9b59d0", 5: "#e08a1e", 6: "#d64545", 7: "#e05bb5"}
STATE_COLORS = {"shop": "#e0b830", "board": "#4cbb5c", "owned": "#4cbb5c", "buy": "#7fe07f",
                "hand": "#4a90d9", "next": "#e0b830", "me": "#4cbb5c", "playing": "#e0b830",
                "selected": "#ffffff"}
ATTACK, HEALTH = "#d9a521", "#c43c3c"


def plain_text(text):
    """Texte de carte sans balises HTML (<b>, [x]...) pour l'infobulle."""
    return re.sub(r"<[^>]+>|\[x\]", "", text or "").replace("\n", " ").strip()


def tier_color(tier):
    return TIER_COLORS.get(tier, "#5c606b")


def place_color(avg):
    """Couleur d'un placement moyen : vert = très bon, jaune = moyen, rouge = faible."""
    if avg is None:
        return "#5c606b"
    return "#3f9d55" if avg < 3.4 else "#b8962a" if avg < 3.8 else "#b5473f"


def pill_html(text, color, fg="#ffffff"):
    return (f"<span style='background:{color};color:{fg};font-weight:bold'>&nbsp;{text}&nbsp;</span>")


def _shadow_text(p, rect, flags, text, color="#ffffff"):
    p.setPen(QColor(0, 0, 0, 220))
    for dx, dy in ((1, 1), (-1, 1), (1, -1), (0, 2)):
        p.drawText(rect.translated(dx, dy), flags, text)
    p.setPen(QColor(color))
    p.drawText(rect, flags, text)


class CardPreview(QLabel):
    """Fenêtre sans cadre avec l'image de la carte survolée (comme HDT). Une seule pour toute l'appli."""
    HEIGHT = 330

    def __init__(self, images):
        super().__init__()
        self.images = images                   # TileCache des images entières (tiles.CARD_URLS)
        self.setWindowFlags(Qt.ToolTip | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint)
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setAttribute(Qt.WA_ShowWithoutActivating)
        self.card_id, self.anchor = None, None
        images.ready.connect(self._arrived)

    def show_for(self, card_id, anchor):
        self.card_id, self.anchor = card_id, anchor
        pix = self.images.get(card_id)
        if pix is None or pix.isNull():
            self.hide()                        # pas encore téléchargée : montrée à son arrivée
            return
        pix = pix.scaledToHeight(self.HEIGHT, Qt.SmoothTransformation)
        self.setPixmap(pix)
        self.resize(pix.size())
        # À côté de la fenêtre de la tuile, du côté du centre de l'écran, à la hauteur de la tuile.
        win = anchor.window().frameGeometry()
        screen = (anchor.screen() or QApplication.primaryScreen()).availableGeometry()
        x = win.left() - pix.width() - 6 if win.center().x() > screen.center().x() else win.right() + 6
        y = anchor.mapToGlobal(anchor.rect().center()).y() - pix.height() // 2
        y = max(screen.top(), min(y, screen.bottom() - pix.height()))
        self.move(x, y)
        self.show()

    def leave(self, card_id):
        if card_id == self.card_id:
            self.card_id = None
            self.hide()

    def _arrived(self, card_id):
        if card_id == self.card_id and self.anchor is not None and self.anchor.underMouse():
            self.show_for(card_id, self.anchor)


PREVIEW = None          # CardPreview posé par la fenêtre principale (ui.py) ; None = pas d'aperçu


def install_preview(images):
    global PREVIEW
    PREVIEW = CardPreview(images)
    return PREVIEW


class Tile(QWidget):
    """Ligne façon HDT. badges = [(texte, couleur), ...] dessinés de droite à gauche ; round=True pour
    des ronds (attaque / vie) sinon des étiquettes arrondies."""
    HEIGHT = 30

    def __init__(self, name, art=None, gem=None, gem_color=None, badges=(), state="", mark="",
                 golden=False, dim=False, tooltip="", height=None, card_id=None):
        super().__init__()
        self.card_id = card_id                 # carte (ou héros) à montrer en grand au survol
        self.name, self.art, self.gem, self.gem_color = name, art, gem, gem_color
        self.badges, self.state, self.mark, self.golden, self.dim = list(badges), state, mark, golden, dim
        self.setFixedHeight(height or self.HEIGHT)
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.setMinimumWidth(120)
        # Avec l'aperçu de la carte, l'infobulle texte ferait doublon : on ne la garde que sans image.
        self.tooltip = tooltip
        if tooltip and not card_id:
            self.setToolTip(tooltip)

    def enterEvent(self, event):
        if self.card_id and PREVIEW is not None:
            PREVIEW.show_for(self.card_id, self)
            if PREVIEW.isHidden() and self.tooltip:
                self.setToolTip(self.tooltip)      # image pas encore là : le texte en attendant
        super().enterEvent(event)

    def leaveEvent(self, event):
        if PREVIEW is not None:
            PREVIEW.leave(self.card_id)
        super().leaveEvent(event)

    def hideEvent(self, event):
        if PREVIEW is not None and self.card_id:
            PREVIEW.leave(self.card_id)
        super().hideEvent(event)

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.setRenderHint(QPainter.SmoothPixmapTransform)
        w, h = self.width(), self.height()
        frame = QRectF(0.5, 0.5, w - 1, h - 1)
        clip = QPainterPath()
        clip.addRoundedRect(frame, 4, 4)
        p.fillPath(clip, BG)
        # Image de la carte à droite, fondue vers la gauche dans le fond (comme les bandeaux de HDT).
        if self.art is not None and not self.art.isNull():
            p.save()
            p.setClipPath(clip)
            art = self.art.scaledToHeight(h, Qt.SmoothTransformation)
            # Le bord gauche des bandeaux est souvent clair : on n'en garde que les 70 % de droite.
            art = art.copy(int(art.width() * 0.3), 0, art.width() - int(art.width() * 0.3), h)
            x0 = max(w * 0.30, w - art.width() * 1.25)
            art = art.scaled(int(w - x0), h, Qt.KeepAspectRatioByExpanding, Qt.SmoothTransformation)
            p.drawPixmap(int(x0), 0, art)
            fade = QLinearGradient(x0, 0, x0 + art.width() * 0.75, 0)
            for at, alpha in ((0, 255), (0.25, 230), (0.5, 150), (0.8, 50), (1, 20)):
                c = QColor(BG)
                c.setAlpha(alpha)
                fade.setColorAt(at, c)
            p.fillRect(QRectF(x0 - 1, 0, w - x0 + 1, h), fade)
            p.restore()
        if self.golden:                                   # carte dorée : léger voile doré
            p.fillPath(clip, QColor(224, 184, 48, 40))
        if self.dim:
            p.fillPath(clip, QColor(0, 0, 0, 120))
        # Contour selon l'état
        border = STATE_COLORS.get(self.state)
        p.setPen(QPen(QColor(border), 2) if border else QPen(QColor("#2c2f39"), 1))
        p.drawPath(clip)
        # Pastille à gauche (tier, place...)
        x = 4
        if self.gem is not None:
            d = h - 8
            r = QRectF(x, 4, d, d)
            p.setPen(QPen(QColor(0, 0, 0, 160), 1.5))
            p.setBrush(QColor(self.gem_color or "#5c606b"))
            p.drawEllipse(r)
            f = QFont("Segoe UI", 9)
            f.setBold(True)
            p.setFont(f)
            _shadow_text(p, r, Qt.AlignCenter, str(self.gem))
            x += d + 6
        # Badges à droite
        right = w - 4
        f = QFont("Segoe UI", 8)
        f.setBold(True)
        p.setFont(f)
        fm = QFontMetrics(f)
        for text, color in reversed(self.badges):
            text = str(text)
            bw = max(h - 10, fm.horizontalAdvance(text) + 10)
            r = QRectF(right - bw, 5, bw, h - 10)
            p.setPen(QPen(QColor(0, 0, 0, 170), 1))
            p.setBrush(QColor(color))
            p.drawRoundedRect(r, (h - 10) / 2, (h - 10) / 2)
            _shadow_text(p, r, Qt.AlignCenter, text)
            right -= bw + 3
        # Nom (avec marque devant : ★ comp, ▶ achat...)
        f = QFont("Segoe UI", 9)
        f.setBold(True)
        p.setFont(f)
        fm = QFontMetrics(f)
        label = (self.mark + " " if self.mark else "") + self.name
        label = fm.elidedText(label, Qt.ElideRight, int(right - x - 4))
        color = "#f0d27a" if self.golden else "#9aa0ad" if self.dim else "#ffffff"
        _shadow_text(p, QRectF(x, 0, right - x, h), Qt.AlignVCenter | Qt.AlignLeft, label, color)
        p.end()


def card_tile(card, tiles, art_on=True, state="", mark="", attack=None, health=None, golden=False,
              dim=False, extra=()):
    """Tuile d'une carte du jeu : pastille = tier, badges = attaque / vie (ou coût d'un sort)."""
    if card.is_pool_spell:
        badges = [(f"{card.cost}g", "#8a6d1e")]
    else:
        badges = [(card.attack if attack is None else attack, ATTACK),
                  (card.health if health is None else health, HEALTH)]
    return Tile(card.name, tiles.get(card.id) if art_on else None, gem=card.tier or "?",
                gem_color=tier_color(card.tier), badges=list(extra) + badges, state=state, mark=mark,
                golden=golden, dim=dim, tooltip=plain_text(card.text), card_id=card.id)


class Thumbs(QWidget):
    """Rangée de petites images de cartes (avec leur tier) ; contour vert = je l'ai."""
    H = 26

    def __init__(self, cards, tiles, owned=(), shop=(), art_on=True, max_count=6):
        super().__init__()
        self.cards = list(cards)[:max_count]
        self.more = max(0, len(cards) - max_count)
        self.tiles, self.owned, self.shop, self.art_on = tiles, set(owned), set(shop), art_on
        self.setFixedHeight(self.H)
        self.setToolTip(", ".join(c.name for c in cards))

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.setRenderHint(QPainter.SmoothPixmapTransform)
        n = len(self.cards) + (1 if self.more else 0)
        if not n:
            return
        gap = 3
        tw = min(58, (self.width() - gap * (n - 1)) / n)
        x = 0.0
        for c in self.cards:
            r = QRectF(x, 0, tw, self.H)
            path = QPainterPath()
            path.addRoundedRect(r, 3, 3)
            p.fillPath(path, BG)
            art = self.tiles.get(c.id) if self.art_on else None
            if art is not None and not art.isNull():
                p.save()
                p.setClipPath(path)
                scaled = art.scaledToHeight(self.H, Qt.SmoothTransformation)
                # Partie centrale-droite du bandeau : c'est là qu'est le visage du serviteur.
                sx = max(0, scaled.width() - tw - scaled.width() * 0.12)
                p.drawPixmap(r.toRect(), scaled, QRectF(sx, 0, tw, self.H).toRect())
                p.restore()
            else:
                f = QFont("Segoe UI", 7)
                p.setFont(f)
                p.setPen(QColor("#c0c4cc"))
                p.drawText(r.adjusted(2, 0, -2, 0), Qt.AlignCenter | Qt.TextWordWrap, c.name)
            state = "owned" if c.id in self.owned else "shop" if c.id in self.shop else ""
            p.setPen(QPen(QColor(STATE_COLORS[state]), 2) if state else QPen(QColor("#2c2f39"), 1))
            p.setBrush(Qt.NoBrush)
            p.drawPath(path)
            # tier en bas à gauche
            g = QRectF(x + 2, self.H - 13, 11, 11)
            p.setPen(Qt.NoPen)
            p.setBrush(QColor(tier_color(c.tier)))
            p.drawEllipse(g)
            f = QFont("Segoe UI", 6)
            f.setBold(True)
            p.setFont(f)
            p.setPen(QColor("#ffffff"))
            p.drawText(g, Qt.AlignCenter, str(c.tier or "?"))
            x += tw + gap
        if self.more:
            r = QRectF(x, 0, min(tw, 30), self.H)
            p.setPen(QColor("#9aa0ad"))
            p.drawText(r, Qt.AlignCenter, f"+{self.more}")
        p.end()
