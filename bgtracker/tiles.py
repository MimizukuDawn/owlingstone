"""Images des cartes dans la liste (comme les bandeaux de HDT).

Qui fait quoi :
- HearthstoneJSON publie un petit bandeau (tile) par carte : art.hearthstonejson.com/v1/tiles/<id>.png.
- TileCache.get(id) renvoie l'image prête à dessiner, ou None si elle n'est pas encore là. Dans ce cas
  il la télécharge en arrière-plan (sans bloquer la fenêtre), l'enregistre dans tiles/ (dossier USER_DIR) et émet
  le signal `ready` : la fenêtre redessine alors la liste. Hors ligne : pas d'image, rien ne casse.
- La même classe sert aux images entières des cartes (CARD_URLS, dossier cards/) montrées au survol.
- L'image est dessinée et fondue par les tuiles de widgets.py (comme dans HDT).
"""
from PySide6.QtCore import QObject, QUrl, Signal
from PySide6.QtGui import QPixmap
from PySide6.QtNetwork import QNetworkAccessManager, QNetworkReply, QNetworkRequest

from .paths import USER_DIR

TILES_URL = "https://art.hearthstonejson.com/v1/tiles/{}.png"
TILES_DIR = USER_DIR / "tiles"
# Image entière de la carte (montrée au survol, comme HDT) : version Battlegrounds (avec le tier), sinon
# l'image normale de la carte (héros, sorts...).
CARD_URLS = ["https://art.hearthstonejson.com/v1/bgs/latest/enUS/256x/{}.png",
             "https://art.hearthstonejson.com/v1/render/latest/enUS/256x/{}.png"]
CARDS_DIR = USER_DIR / "cards"


class TileCache(QObject):
    ready = Signal(str)          # id de la carte dont l'image vient d'arriver

    def __init__(self, folder=TILES_DIR, parent=None, urls=(TILES_URL,)):
        super().__init__(parent)
        self.folder = folder
        self.urls = list(urls)       # adresses essayées dans l'ordre
        self.images = {}         # id -> QPixmap (le bandeau tel quel, 256×59 ; widgets.py le fond)
        self.pending = set()     # téléchargements en cours
        self.failed = set()      # pas d'image (hors ligne, carte sans bandeau) : on ne réessaie pas
        self.net = QNetworkAccessManager(self)

    def get(self, card_id):
        if card_id in self.images:
            return self.images[card_id]
        path = self.folder / f"{card_id}.png"
        if path.exists():
            pix = QPixmap(str(path))
            if not pix.isNull():
                self.images[card_id] = pix
                return self.images[card_id]
        if card_id not in self.pending and card_id not in self.failed:
            self.pending.add(card_id)
            self._fetch(card_id, 0)
        return None

    def _fetch(self, card_id, n):
        reply = self.net.get(QNetworkRequest(QUrl(self.urls[n].format(card_id))))
        reply.finished.connect(lambda r=reply, i=card_id, n=n: self._done(r, i, n))

    def _done(self, reply, card_id, n=0):
        data = bytes(reply.readAll()) if reply.error() == QNetworkReply.NoError else b""
        reply.deleteLater()
        pix = QPixmap()
        if not data or not pix.loadFromData(data):
            if n + 1 < len(self.urls):
                self._fetch(card_id, n + 1)       # adresse suivante
                return
            self.pending.discard(card_id)
            self.failed.add(card_id)
            return
        self.pending.discard(card_id)
        try:
            self.folder.mkdir(parents=True, exist_ok=True)
            (self.folder / f"{card_id}.png").write_bytes(data)
        except OSError:
            pass                 # pas de place / dossier en lecture seule : l'image reste en mémoire
        self.images[card_id] = pix
        self.ready.emit(card_id)
