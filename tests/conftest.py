"""Réglages communs des tests : jamais de téléchargement de cartes (la copie incluse suffit, et la CI
ne dépend pas de HearthstoneJSON)."""
import os

os.environ.setdefault("OWLINGSTONE_NO_CARD_DOWNLOAD", "1")
