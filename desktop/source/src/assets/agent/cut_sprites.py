"""Cut the magenta sprite sheet into transparent PNG frames for the AI-at-work page."""
import json
from pathlib import Path
from PIL import Image

SRC = Path(__file__).with_name("sheet.webp")
OUT = Path(__file__).resolve().parent
OUT.mkdir(parents=True, exist_ok=True)
sheet = Image.open(SRC).convert("RGBA")


def magenta(p) -> bool:
    r, g, b, _ = p
    return r > 120 and b > 120 and g < 130 and r - g > 55 and b - g > 55


def cut(box) -> Image.Image:
    img = sheet.crop(box)
    px = img.load()
    for y in range(img.height):
        for x in range(img.width):
            if magenta(px[x, y]):
                px[x, y] = (0, 0, 0, 0)
    return img


def trim(img: Image.Image) -> Image.Image:
    return img.crop(img.getbbox())


# Standing / walking frames (grid found by scanning the sheet), padded to one canvas, feet at the bottom centre.
walk = {
    "front_stand": (116, 29, 257, 281), "front_a": (417, 29, 554, 281), "front_b": (724, 29, 862, 281),
    "back_stand": (120, 298, 256, 607), "back_a": (418, 298, 548, 607), "back_b": (724, 298, 858, 607),
    "side_a": (105, 609, 248, 869), "side_b": (421, 609, 559, 869), "side_carry": (734, 609, 882, 869),
}
frames = {name: trim(cut(box)) for name, box in walk.items()}
cw = max(f.width for f in frames.values()) + 4
ch = max(f.height for f in frames.values()) + 2
meta = {"walk": {"w": cw, "h": ch}, "seated": {}}
for name, f in frames.items():
    canvas = Image.new("RGBA", (cw, ch), (0, 0, 0, 0))
    canvas.paste(f, ((cw - f.width) // 2, ch - f.height), f)
    canvas.save(OUT / f"{name}.png", optimize=True)

# Seated frames: keep the character and the open book above the desk edge (y≈1080), drop the sheet's own desk, lamp and book pile.
DESK = 1079
seated = {"read": (92, 893, 232), "think": (416, 893, 540), "cheer": (686, 880, 880), "sleep": (1040, 880, 1218)}
# Props from the sheet's own desk that reach into a crop: (x0, y0, x1, y1) in sheet coordinates.
ERASE = {"cheer": [(686, 1010, 752, 1079), (862, 955, 880, 1079)], "sleep": [(1178, 965, 1218, 1079), (1040, 1010, 1058, 1079)], "think": [(416, 1010, 426, 1079)]}
for name, (x0, y0, x1) in seated.items():
    img = cut((x0, y0, x1, DESK))
    px = img.load()
    for ex0, ey0, ex1, ey1 in ERASE.get(name, []):
        for y in range(max(0, ey0 - y0), min(img.height, ey1 - y0)):
            for x in range(max(0, ex0 - x0), min(img.width, ex1 - x0)):
                px[x, y] = (0, 0, 0, 0)
    box = img.getbbox()
    img = img.crop((box[0], box[1], box[2], img.height))
    # Horizontal anchor = centre of the hair in the top 70 rows (the head), so frames line up on the chair.
    hair = [x for x in range(img.width) for y in range(min(70, img.height)) if img.getpixel((x, y))[3] and sum(img.getpixel((x, y))[:3]) < 260]
    cx = (min(hair) + max(hair)) / 2 if hair else img.width / 2
    img.save(OUT / f"seat_{name}.png", optimize=True)
    meta["seated"][name] = {"w": img.width, "h": img.height, "cx": round(cx, 1)}

(OUT / "frames.json").write_text(json.dumps(meta, indent=1), encoding="utf-8")
print(json.dumps(meta, indent=1))
print({p.name: p.stat().st_size for p in OUT.glob("*.png")})
