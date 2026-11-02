# PyInstaller build of Owlingstone, used by update.bat and the release workflow.
#   pyinstaller --noconfirm Owlingstone.spec                       -> dist\Owlingstone\ (folder)
#   set OWLINGSTONE_ONEFILE=1 & pyinstaller --noconfirm Owlingstone.spec  -> dist\Owlingstone.exe (one file)
# Parts of Qt the app never uses are left out (QML/Quick, PDF, software OpenGL, virtual keyboard,
# translations...): about half the size, so a smaller download and a faster start. The release workflow
# starts the built exe once (OWLINGSTONE_SMOKE) to prove nothing it needs was removed.
import os
from pathlib import Path

ROOT = Path(SPECPATH)
ONEFILE = os.environ.get("OWLINGSTONE_ONEFILE") == "1"

UNUSED = {
    "opengl32sw.dll", "qt6quick.dll", "qt6qml.dll", "qt6qmlmodels.dll", "qt6qmlmeta.dll",
    "qt6qmlworkerscript.dll", "qt6pdf.dll", "qt6opengl.dll", "qt6virtualkeyboard.dll",
    "qpdf.dll", "qtvirtualkeyboardplugin.dll", "qdirect2d.dll", "qtiff.dll", "qicns.dll", "qtga.dll",
    "qwbmp.dll", "qtuiotouchplugin.dll", "qopensslbackend.dll", "libcrypto-3-x64.dll", "libssl-3-x64.dll",
}


def keep(entry):
    dest = entry[0].replace("\\", "/").lower()
    return dest.rsplit("/", 1)[-1] not in UNUSED and "/translations/" not in "/" + dest


a = Analysis(
    [str(ROOT / "Owlingstone.pyw")],
    pathex=[str(ROOT)],
    datas=[(str(ROOT / "bgtracker" / "icon.ico"), "bgtracker"),
           (str(ROOT / "data" / "*.yaml"), "data"),
           (str(ROOT / "data" / "cards-fallback.json.gz"), "data"),
           (str(ROOT / "CHANGELOG.md"), ".")],
    excludes=["tkinter", "PySide6.QtQml", "PySide6.QtQuick", "PySide6.QtOpenGL"],
)
a.binaries = [b for b in a.binaries if keep(b)]
a.datas = [d for d in a.datas if keep(d)]
pyz = PYZ(a.pure)
icon = str(ROOT / "bgtracker" / "icon.ico")

if ONEFILE:
    exe = EXE(pyz, a.scripts, a.binaries, a.datas, name="Owlingstone", icon=icon, console=False, upx=False)
else:
    exe = EXE(pyz, a.scripts, exclude_binaries=True, name="Owlingstone", icon=icon, console=False, upx=False)
    coll = COLLECT(exe, a.binaries, a.datas, name="Owlingstone", upx=False)
