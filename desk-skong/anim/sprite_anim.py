from __future__ import annotations
import json
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

from PIL import Image


@dataclass
class PixelRect:
    x: int
    y: int
    w: int
    h: int


@dataclass
class Size2D:
    w: int
    h: int


@dataclass
class Offset2D:
    x: int
    y: int


@dataclass
class SpriteDef:
    name: str
    pixel_rect: PixelRect  # in atlas, bottom-left origin
    flip: str  # "None" | "TPackerCW" (90° CW) | "Tk2d" (90° CW + H-flip)
    source_size: Optional[Size2D] = None  # untrimmed canvas size (pixels)
    sprite_source_offset: Optional[Offset2D] = (
        None  # bottom-left offset of trimmed rect inside the canvas
    )


@dataclass
class SkinData:
    atlas_width: int
    atlas_height: int
    sprites: List[SpriteDef]
    name_to_index: Dict[str, int]


@dataclass(frozen=True)
class DrawPacket:
    """
    Ready-to-render data for the *current frame*:
      - `image`: PIL image already rotated/flipped and scaled
      - `scaled_canvas_size`: (W,H) of the untrimmed canvas after scaling
      - `scaled_frame_rect_tl`: (x,y,w,h) top-left rect of the frame inside the canvas (after scaling)
      - `name`: frame name (useful for logs)
      - `scale`: the scale applied
    """

    name: str
    image: Image.Image
    scaled_canvas_size: Tuple[int, int]
    scaled_frame_rect_tl: Tuple[int, int, int, int]
    scale: float


# ========= Skin loader (atlas + metadata) =========


class Skin:
    """
    Loads the tk2d-exported JSON + atlas PNG and exposes sprite lookups/crops.

    Coordinate conventions:
      - tk2d pixelRect.y is **bottom-left origin**.
      - PIL crops use **top-left origin**; we convert for you.
      - sprite_source_offset is bottom-left; we convert to top-left draw offset.
    """

    def __init__(self, data: SkinData, atlas: Image.Image):
        self.data = data
        self.atlas = atlas.convert("RGBA")

    @classmethod
    def load(cls, tk2d_json_path: str, atlas_png_path: str) -> "Skin":
        with open(tk2d_json_path, "r", encoding="utf-8") as f:
            obj = json.load(f)

        sprites: List[SpriteDef] = []
        name_to_index: Dict[str, int] = {}

        for i, s in enumerate(obj["Sprites"]):
            pr = s["PixelRect"]
            pixel_rect = PixelRect(
                int(pr["X"]), int(pr["Y"]), int(pr["W"]), int(pr["H"])
            )

            ss = s.get("SourceSize")
            source_size = Size2D(int(ss["W"]), int(ss["H"])) if ss else None

            so = s.get("SpriteSourceOffset")
            sprite_source_offset = Offset2D(int(so["X"]), int(so["Y"])) if so else None

            flip = s.get("Flip", "None")

            sd = SpriteDef(
                name=s["Name"],
                pixel_rect=pixel_rect,
                flip=flip,
                source_size=source_size,
                sprite_source_offset=sprite_source_offset,
            )
            sprites.append(sd)
            name_to_index[sd.name] = i

        atlas = Image.open(atlas_png_path)
        data = SkinData(
            atlas_width=int(obj["AtlasWidth"]),
            atlas_height=int(obj["AtlasHeight"]),
            sprites=sprites,
            name_to_index=name_to_index,
        )
        return cls(data, atlas)

    def sprite_index(self, name: str) -> Optional[int]:
        return self.data.name_to_index.get(name)

    def names(self):
        return self.data.name_to_index.keys()

    # ---- cropping + transforms ----

    def _crop_pil_from_atlas(self, pr: PixelRect) -> Image.Image:
        """
        Crop a sub-image from the atlas.
        tk2d Y is bottom-left; PIL cropping expects top-left.
        """
        W, H = self.atlas.size
        x = max(0, pr.x)
        y_top = max(0, H - (pr.y + pr.h))
        w = max(1, pr.w)
        h = max(1, pr.h)
        # clamp in-bounds
        w = min(w, W - x)
        h = min(h, H - y_top)
        return self.atlas.crop((x, y_top, x + w, y_top + h))

    @staticmethod
    def _apply_flip(img: Image.Image, flip: str) -> Image.Image:
        """
        Texture rotation semantics:
          - "TPackerCW" => rotate 90° CW (PIL: ROTATE_270)
          - "Tk2d"     => rotate 90° CW, then horizontal flip
          - "None"     => no change
        """
        if flip == "None":
            return img
        if flip == "TPackerCW":
            return img.transpose(Image.ROTATE_270)
        if flip == "Tk2d":
            return img.transpose(Image.ROTATE_90).transpose(Image.FLIP_LEFT_RIGHT)
        # unknown -> no-op
        return img

    def frame_image(self, sd: SpriteDef) -> Image.Image:
        """Return a PIL image for the (upright) frame, with tk2d rotation applied."""
        sub = self._crop_pil_from_atlas(sd.pixel_rect)
        return self._apply_flip(sub, sd.flip)

    def canvas_size(
        self, sd: SpriteDef, frame_img: Optional[Image.Image] = None
    ) -> Tuple[int, int]:
        """
        Untrimmed canvas size (pixels).
        If SourceSize is present, it defines the canvas.
        Otherwise, use the size of the ROTATION-ADJUSTED frame (post-flip) so bounds don't stay sideways.
        """
        if sd.source_size:
            return sd.source_size.w, sd.source_size.h
        if frame_img is None:
            frame_img = self.frame_image(sd)  # applies rotation/flip
        return frame_img.width, frame_img.height

    def top_left_offset_in_canvas(
        self, sd: SpriteDef, frame_img: Image.Image
    ) -> Tuple[int, int]:
        """
        Where to draw the (rotated) frame inside its untrimmed canvas in **top-left** coords.
        tk2d stores sprite_source_offset in bottom-left coords relative to the canvas.
        For top-left, y_top = canvas_h - (off_y + frame_h).
        """
        _, ch = self.canvas_size(sd, frame_img)
        if sd.sprite_source_offset:
            off_x = max(0, sd.sprite_source_offset.x)
            off_y = max(0, sd.sprite_source_offset.y)
        else:
            off_x = 0
            off_y = 0
        # bottom-left → top-left
        y_top = max(0, ch - (off_y + frame_img.height))
        x_left = max(0, off_x)
        return x_left, y_top


# ========= Animation config + player with transitions =========


@dataclass
class AnimGroup:
    name: str
    fps: float
    loop: bool
    order: Optional[List[str]] = None
    prefix: Optional[str] = None
    transition_frames: Optional[Dict[str, str]] = None  # e.g., {"run": "idle_to_run"}


class AnimationConfig:
    def __init__(self, groups: Dict[str, AnimGroup]):
        self.groups = groups

    @classmethod
    def load_file(cls, path: str) -> "AnimationConfig":
        with open(path, "r", encoding="utf-8") as f:
            raw = json.load(f)
        groups: Dict[str, AnimGroup] = {}
        for g in raw["groups"]:
            groups[g["name"]] = AnimGroup(
                name=g["name"],
                fps=float(g.get("fps", 30.0)),
                loop=bool(g.get("loop", True)),
                order=g.get("order"),
                prefix=g.get("prefix"),
                transition_frames=g.get("transition_frames", {}),
            )
        return cls(groups)

    def get(self, name: str) -> Optional[AnimGroup]:
        return self.groups.get(name)


class SpritePlayer:
    """
    Time-based player that:
      - resolves group -> list of sprite indices (by order or prefix),
      - ticks by dt seconds at group's fps,
      - loops or clamps,
      - supports transition groups (one-shot) before switching to target.

    Use:
        skin = Skin.load("data/Knight_tk2d.json", "data/Knight.png")
        anims = AnimationConfig.load_file("data/animations.json")
        sp = SpritePlayer(skin, anims, start_group="idle")

        # game/update loop
        sp.update(dt)
        frame = sp.current_frame()  # PIL image to draw
        canvas_w, canvas_h = sp.current_canvas_size()
        ox, oy = sp.current_top_left_offset()  # where to blit inside canvas (top-left)
    """

    def __init__(self, skin: Skin, config: AnimationConfig, start_group: str):
        self.skin = skin
        self.config = config

        self.current_group = start_group
        self.pending_after_transition: Optional[str] = None

        self._seq: List[int] = self._resolve_group(start_group)
        g = self.config.get(start_group)
        if not g:
            raise ValueError(f"Unknown group '{start_group}'")
        self.fps = max(1e-3, g.fps)
        self.loop = bool(g.loop)

        self.accum = 0.0
        self.index = 0

    # ---- group resolution & switching ----

    def _resolve_group(self, name: str) -> List[int]:
        g = self.config.get(name)
        if not g:
            raise ValueError(f"Unknown group '{name}'")
        out: List[int] = []
        if g.order:
            for nm in g.order:
                idx = self.skin.sprite_index(nm)
                if idx is None:
                    raise ValueError(f"Frame not found in skin: {nm}")
                out.append(idx)
        elif g.prefix:
            names = sorted([n for n in self.skin.names() if n.startswith(g.prefix)])
            if not names:
                raise ValueError(f"No sprites found with prefix '{g.prefix}'")
            for nm in names:
                out.append(self.skin.sprite_index(nm))
        else:
            raise ValueError(f"AnimGroup '{name}' lacks 'order' or 'prefix'")
        return out

    def _apply_group(
        self, name: str, loop: Optional[bool] = None, fps: Optional[float] = None
    ):
        self._seq = self._resolve_group(name)
        g = self.config.get(name)
        self.loop = g.loop if loop is None else bool(loop)
        self.fps = g.fps if fps is None else float(fps)
        self.current_group = name
        self.accum = 0.0
        self.index = 0

    def goto(self, target: str):
        """Switch to target, optionally passing through a one-shot transition."""
        if self.current_group == target:
            return
        cur_g = self.config.get(self.current_group)
        used_transition = False
        if cur_g and cur_g.transition_frames and target in cur_g.transition_frames:
            trans_name = cur_g.transition_frames[target]
            tg = self.config.get(trans_name)
            if tg:
                self._apply_group(trans_name, loop=False)  # one-shot
                self.pending_after_transition = target
                used_transition = True
        if not used_transition:
            self._apply_group(target)
            self.pending_after_transition = None

    # ---- ticking ----

    def update(self, dt: float):
        """Advance by dt seconds; handle transition completion."""
        self.accum += dt * self.fps
        adv = int(self.accum)
        if adv > 0:
            self.accum -= adv
            self.index += adv
            if self.loop:
                self.index %= len(self._seq)
            else:
                if self.index >= len(self._seq):
                    self.index = len(self._seq) - 1
                    # if we finished a transition, jump to pending target
                    if self.pending_after_transition:
                        target = self.pending_after_transition
                        self.pending_after_transition = None
                        self._apply_group(target)

    # ---- single source of truth for drawing/collisions ----

    def draw_packet(self, scale: float = 1.0) -> DrawPacket:
        """
        Returns everything a renderer needs to draw the frame bottom-aligned and to
        compute a sane default collision rect (the frame rect inside the canvas).
        """
        sd = self.skin.data.sprites[self._seq[self.index]]

        # 1) rotate/crop frame to be upright
        img = self.skin.frame_image(sd)  # already applies Flip (TPackerCW, Tk2d, None)

        # 2) rotation-aware canvas size
        cw, ch = self.skin.canvas_size(
            sd, img
        )  # uses SourceSize if present, else frame.size

        # 3) top-left offset of the (trimmed) frame inside the (untrimmed) canvas
        ox, oy = self.skin.top_left_offset_in_canvas(sd, img)

        # 4) apply scale consistently to everything
        if scale != 1.0:
            sw = max(1, int(round(img.width * scale)))
            sh = max(1, int(round(img.height * scale)))
            img = img.resize((sw, sh), Image.NEAREST)
            cw = int(round(cw * scale))
            ch = int(round(ch * scale))
            ox = int(round(ox * scale))
            oy = int(round(oy * scale))

        return DrawPacket(
            name=sd.name,
            image=img,
            scaled_canvas_size=(cw, ch),
            scaled_frame_rect_tl=(ox, oy, img.width, img.height),
            scale=float(scale),
        )
