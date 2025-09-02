# demo.py
import time
from anim.sprite_anim import Skin, AnimationConfig, SpritePlayer
from tools.sprite_viewer import main as main_renderer

def demo() :
    skin = Skin.load("data/Knight_tk2d.json", "data/Knight.png")
    anims = AnimationConfig.load_file("data/animations.json")
    player = SpritePlayer(skin, anims, start_group="idle")

    t0 = time.perf_counter()
    last = t0

    for _ in range(120):  # ~2s at 60fps
        now = time.perf_counter()
        dt = now - last
        last = now

        player.update(dt)
        pkt = player.draw_packet(scale=0.5)
        print(f"{pkt['name']:>32}  size={pkt['scaled_size']}  offTL={pkt['scaled_offset_tl']}  canvas={pkt['canvas_size']}")
        time.sleep(1/60)

    # Try a transition: idle -> run
    player.goto("run")
    for _ in range(180):
        now = time.perf_counter()
        dt = now - last
        last = now
        player.update(dt)
        print(player.frame_name())
        time.sleep(1/60)


if __name__ == '__main__' :
    main_renderer()