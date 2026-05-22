#!/usr/bin/env python3
"""Render the title card as a PNG using Pillow."""
from PIL import Image, ImageDraw, ImageFont

W, H = 1080, 1920
FONT = "/System/Library/Fonts/Supplemental/Impact.ttf"

img = Image.new("RGB", (W, H), (0, 0, 0))
draw = ImageDraw.Draw(img)

# Tape stamp top-left
mono = "/System/Library/Fonts/Menlo.ttc"
try:
    f_mono = ImageFont.truetype(mono, 28)
except Exception:
    f_mono = ImageFont.load_default()
draw.text((52, 52), "● REC   SP   LP-007", fill=(225, 60, 60), font=f_mono)

# "— DRINK —" small caps marker
f_mark = ImageFont.truetype(mono, 28)
mark = "—   D R I N K   —"
mbox = draw.textbbox((0, 0), mark, font=f_mark)
mw = mbox[2] - mbox[0]
draw.text(((W - mw) // 2, int(H * 0.22)), mark, fill=(244, 244, 238), font=f_mark)

# LIQUID DEATH wordmark — heavy chiseled
# Pillow doesn't support font-stretch, so we draw text to an off-screen layer
# then resize vertically to mimic the chiseled metal-band proportions.
def draw_stretched(text, size, y_center, stretch_y=1.20, color=(244, 244, 238), shadow=True):
    f = ImageFont.truetype(FONT, size)
    bb = draw.textbbox((0, 0), text, font=f)
    tw = bb[2] - bb[0]
    th = bb[3] - bb[1]
    pad = 40
    layer = Image.new("RGBA", (tw + pad * 2, th + pad * 2), (0, 0, 0, 0))
    ld = ImageDraw.Draw(layer)
    if shadow:
        ld.text((pad, pad + 8), text, fill=(20, 20, 20, 255), font=f)
    ld.text((pad, pad), text, fill=color + (255,), font=f)
    new_h = int(layer.size[1] * stretch_y)
    layer = layer.resize((layer.size[0], new_h), Image.LANCZOS)
    paste_x = (W - layer.size[0]) // 2
    paste_y = y_center - layer.size[1] // 2
    img.paste(layer, (paste_x, paste_y), layer)

draw_stretched("LIQUID", 340, int(H * 0.38), stretch_y=1.18)
draw_stretched("DEATH",  340, int(H * 0.54), stretch_y=1.18)

# Horizontal rule
rule_y = int(H * 0.65)
rule_w = int(W * 0.66)
draw.rectangle([((W - rule_w) // 2, rule_y), ((W + rule_w) // 2, rule_y + 6)], fill=(244, 244, 238))

# CTA
draw_stretched("MURDER YOUR THIRST", 108, int(H * 0.72), stretch_y=1.10)

# Bottom tape stamp
draw.text((52, H - 90), "● REC   FIN", fill=(225, 60, 60), font=f_mono)

img.save("assets/title-card.png", "PNG")
print("wrote assets/title-card.png", img.size)
