#!/usr/bin/env python3
"""Render captured Flere cells, never invent terminal contents. Requires Pillow."""

import argparse, json, pathlib, unicodedata
from PIL import Image, ImageDraw, ImageFont

BASE = [
    "#242a35",
    "#e5757e",
    "#91c68e",
    "#e3c78a",
    "#83a8de",
    "#bb9fd9",
    "#83c5cc",
    "#d4d9e1",
    "#67778a",
    "#fa8e95",
    "#ade0a8",
    "#f1df9d",
    "#a3c7ef",
    "#d6b9ee",
    "#a6e6ec",
    "#ffffff",
]


def rgb(c, background=False):
    if c == "Default":
        return (12, 12, 12) if background else (204, 204, 204)
    if "Rgb" in c:
        return tuple(c["Rgb"])
    n = c["Index"]
    if n < 16:
        return tuple(int(BASE[n][i : i + 2], 16) for i in (1, 3, 5))
    if n < 232:
        n -= 16
        return tuple(0 if x == 0 else 55 + 40 * x for x in (n // 36, n // 6 % 6, n % 6))
    return (8 + (n - 232) * 10,) * 3


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("capture", type=pathlib.Path)
    p.add_argument("output", type=pathlib.Path)
    p.add_argument(
        "--font",
        required=True,
        help="monospace font file; e.g. Menlo.ttc or DejaVuSansMono.ttf",
    )
    p.add_argument("--intro", action="store_true")
    args = p.parse_args()
    font = ImageFont.truetype(args.font, 16)
    small = ImageFont.truetype(args.font, 12)
    try:
        bold = ImageFont.truetype(args.font, 16, index=1)
    except OSError:
        bold = font
    captures = [json.loads(line) for line in args.capture.read_text().splitlines()]
    images = []
    durations = []
    scenes = {}
    cw, ch = 10, 21
    for i, f in enumerate(captures):
        width, height = f["cols"] * cw + 64, f["rows"] * ch + 136
        image = Image.new("RGB", (width, height), "#070c15")
        d = ImageDraw.Draw(image)
        d.rounded_rectangle(
            (0, 0, width - 1, height - 1), radius=18, outline="#23324a", width=1
        )
        label = f.get("label", "FIRST OPEN / PRESS ANY KEY TO START")
        d.text((32, 22), "FLERE", font=bold, fill="#43e3f7")
        d.text(
            (width - 265, 24),
            f"ACTUAL UI  /  {f['cols']} x {f['rows']}",
            font=small,
            fill="#8093af",
        )
        d.line((32, 54, width - 32, 54), fill="#203049")
        x0, y0 = 32, 72
        for run in f["runs"]:
            if args.intro:
                y, x, text, fg, bg, is_bold = run
                cells = sum(
                    2 if unicodedata.east_asian_width(c) in ("W", "F") else 1
                    for c in text
                )
                style = {"fg": fg, "bg": bg, "bold": is_bold}
            else:
                y, x, cells, text, style = run
            fg, bg = rgb(style["fg"]), rgb(style["bg"], True)
            if style.get("inverse"):
                fg, bg = bg, fg
            if style.get("dim"):
                fg = tuple(int(0.5 * a + 0.5 * b) for a, b in zip(fg, bg))
            left, top = x0 + x * cw, y0 + y * ch
            d.rectangle((left, top, left + cells * cw - 1, top + ch - 1), fill=bg)
            face = bold if style.get("bold") else font
            for c in text:
                joins = {
                    "─": "lr",
                    "━": "lr",
                    "│": "tb",
                    "┃": "tb",
                    "┌": "rb",
                    "┐": "lb",
                    "└": "rt",
                    "┘": "lt",
                    "├": "trb",
                    "┤": "tlb",
                    "┬": "lrb",
                    "┴": "lrt",
                    "┼": "lrtb",
                    "╭": "rb",
                    "╮": "lb",
                    "╰": "rt",
                    "╯": "lt",
                }
                if c in joins:
                    mid = (left + cw // 2, top + ch // 2)
                    for edge in joins[c]:
                        end = {
                            "l": (left, mid[1]),
                            "r": (left + cw, mid[1]),
                            "t": (mid[0], top),
                            "b": (mid[0], top + ch),
                        }[edge]
                        d.line((mid, end), fill=fg, width=2 if c in "━┃" else 1)
                else:
                    d.text((left, top + 16), c, font=face, fill=fg, anchor="ls")
                left += cw * (2 if unicodedata.east_asian_width(c) in ("W", "F") else 1)
            if style.get("underline"):
                d.line(
                    (
                        x0 + x * cw,
                        top + ch - 3,
                        x0 + (x + cells) * cw - 1,
                        top + ch - 3,
                    ),
                    fill=fg,
                )
            if style.get("strikethrough"):
                d.line(
                    (
                        x0 + x * cw,
                        top + ch // 2,
                        x0 + (x + cells) * cw - 1,
                        top + ch // 2,
                    ),
                    fill=fg,
                )
        d.text((32, height - 39), label, font=small, fill="#cfdbf0")
        d.line(
            (32, height - 16, 32 + (width - 64) * (i + 1) / len(captures), height - 16),
            fill="#43e3f7",
            width=2,
        )
        images.append(image)
        durations.append(
            max(20, round((captures[i + 1]["ms"] - f["ms"]) / 10) * 10)
            if i + 1 < len(captures)
            else 800
        )
        scenes[label] = image
    args.output.parent.mkdir(parents=True, exist_ok=True)
    images[0].save(
        args.output,
        save_all=True,
        append_images=images[1:],
        duration=durations,
        loop=0,
        optimize=True,
        disposal=2,
    )
    for i, im in enumerate(scenes.values(), 1):
        im.save(args.output.with_name(f"{args.output.stem}-{i}.png"), optimize=True)
    print(
        f"{args.output}: {len(images)} frames, {sum(durations)/1000:.2f}s, {args.output.stat().st_size:,} bytes"
    )


if __name__ == "__main__":
    main()
