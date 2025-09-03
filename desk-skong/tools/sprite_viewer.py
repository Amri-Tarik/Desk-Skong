# tools/sprite_viewer.py
from __future__ import annotations

import pathlib
import sys

from PySide6 import QtWidgets

# Repo root on path
ROOT = pathlib.Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from anim.sprite_anim import AnimationConfig, Skin
from tools.viewer.scene import Scene, SceneConfig
from tools.viewer.widget import OverlayWidget

# Data paths
SKIN_JSON = ROOT / "data" / "Knight_tk2d.json"
ATLAS_PNG = ROOT / "data" / "Knight.png"
ANIMS_JSON = ROOT / "data" / "animations.json"


def main():
    app = QtWidgets.QApplication(sys.argv)

    # Window size = monitor work area
    screen = QtWidgets.QApplication.primaryScreen()
    if screen is None:
        raise RuntimeError("No primary screen found")
    geo = screen.availableGeometry()
    W, H = geo.width(), geo.height()

    skin = Skin.load(str(SKIN_JSON), str(ATLAS_PNG))
    anims = AnimationConfig.load_file(str(ANIMS_JSON))

    scene = Scene(SceneConfig(width=W, height=H, scale=0.7), skin, anims)
    w = OverlayWidget(scene)
    w.show()

    sys.exit(app.exec())


if __name__ == "__main__":
    main()
