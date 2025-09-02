# demo.py
import time

from anim.sprite_anim import AnimationConfig, Skin, SpritePlayer
from tools.phys_qt_viewer import main as main_combined
from tools.phys_sandbox import main as main_physics_engine
from tools.sprite_viewer import main as main_renderer

if __name__ == "__main__":
    main_renderer()
    # main_physics_engine()
    # main_combined()
