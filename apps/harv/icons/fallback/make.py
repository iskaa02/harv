#!/usr/bin/env python3
"""Generates harv's fallback symbolic icons (original drawings) into this folder.

GTK on macOS (Homebrew) ships no Adwaita icon theme, so every standard icon name the app asks for came up as a broken
image. These are written into the `hicolor` theme the app registers (icons.rs); on Linux, Adwaita sits ahead of hicolor in
the lookup chain and keeps winning, so nothing changes there.

GTK 4.22's symbolic loader only takes plain filled paths (no strokes, no <g>), so the drawings below are made of stroke
primitives that are turned into filled polygons here: every outline has the same winding (nonzero fill), holes the
opposite one. Run `python3 make.py` after editing; the .svg files are what the app embeds.
"""
import math
import os

W = 1.5  # default stroke width (16 px grid)


def area(p):
    return sum(p[i][0] * p[(i + 1) % len(p)][1] - p[(i + 1) % len(p)][0] * p[i][1] for i in range(len(p))) / 2


def orient(p, cw=True):
    """Same winding for every filled shape (positive area in SVG's y-down space), the reverse for holes."""
    return p if (area(p) > 0) == cw else p[::-1]


def circle_pts(cx, cy, r, n=28):
    return [(cx + r * math.cos(2 * math.pi * i / n), cy + r * math.sin(2 * math.pi * i / n)) for i in range(n)]


class Icon:
    def __init__(self):
        self.polys = []

    def poly(self, pts, hole=False):
        self.polys.append(orient(list(pts), not hole))
        return self

    def dot(self, cx, cy, r):
        return self.poly(circle_pts(cx, cy, r, max(12, int(r * 14))))

    def ring(self, cx, cy, r, w=W):
        self.poly(circle_pts(cx, cy, r + w / 2, 32))
        return self.poly(circle_pts(cx, cy, r - w / 2, 32), hole=True)

    def rect(self, x, y, w, h):
        return self.poly([(x, y), (x + w, y), (x + w, y + h), (x, y + h)])

    def seg(self, a, b, w=W, cap=True):
        (x1, y1), (x2, y2) = a, b
        dx, dy = x2 - x1, y2 - y1
        l = math.hypot(dx, dy) or 1
        nx, ny = -dy / l * w / 2, dx / l * w / 2
        self.poly([(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)])
        if cap:
            self.dot(x1, y1, w / 2)
            self.dot(x2, y2, w / 2)
        return self

    def line(self, pts, w=W, close=False):
        pts = list(pts) + ([pts[0]] if close else [])
        for a, b in zip(pts, pts[1:]):
            self.seg(a, b, w)
        return self

    def arc(self, cx, cy, r, a0, a1, w=W, n=None):
        """Arc from a0 to a1 degrees (0 = right, 90 = down)."""
        n = n or max(6, int(abs(a1 - a0) / 12))
        pts = [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)), cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]
        return self.line(pts, w)

    def svg(self):
        d = "".join("M" + " ".join(f"{x:.2f} {y:.2f}" for x, y in p) + "Z" for p in self.polys)
        return f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="{d}"/></svg>\n'


def arrowhead(ic, tip, angle, size=2.6, w=W):
    """Two strokes meeting at `tip`, pointing along `angle` degrees."""
    for s in (-1, 1):
        a = math.radians(angle + 180 + s * 42)
        ic.seg(tip, (tip[0] + size * math.cos(a), tip[1] + size * math.sin(a)), w)


def folder(ic):
    return ic.line([(1.75, 4), (6, 4), (7.5, 5.5), (14.25, 5.5), (14.25, 13), (1.75, 13)], close=True)


def clock(ic, cx=8, cy=8, r=6, hands=True):
    ic.ring(cx, cy, r)
    if hands:
        ic.line([(cx, cy - 3.2), (cx, cy), (cx + 2.4, cy + 1.6)])
    return ic


ICONS = {}


def icon(name):
    def wrap(f):
        ic = Icon()
        f(ic)
        ICONS[name] = ic
        return f
    return wrap


@icon("alarm-symbolic")
def _(ic):
    clock(ic, 8, 9, 5.2)
    ic.arc(3.2, 3.8, 2.2, 200, 300).arc(12.8, 3.8, 2.2, 240, 340)


@icon("audio-volume-high-symbolic")
def _(ic):
    ic.poly([(1.5, 6), (4, 6), (7.5, 2.8), (7.5, 13.2), (4, 10), (1.5, 10)])
    ic.arc(8.2, 8, 3, -50, 50).arc(8.2, 8, 5.6, -55, 55)


@icon("audio-x-generic-symbolic")
def _(ic):
    ic.dot(5, 12, 2.3).dot(12, 10.5, 2.3)
    ic.seg((7, 12), (7, 3.5), 1.6).seg((14, 10.5), (14, 2.2), 1.6).seg((7, 3.5), (14, 2.2), 1.9)


@icon("avatar-default-symbolic")
def _(ic):
    ic.dot(8, 5, 3)
    ic.poly([(8 + 6 * math.cos(math.radians(a)), 15 - 5.6 * math.sin(math.radians(a))) for a in range(0, 181, 12)])


@icon("dialog-error-symbolic")
def _(ic):
    ic.ring(8, 8, 6).seg((5.6, 5.6), (10.4, 10.4)).seg((10.4, 5.6), (5.6, 10.4))


@icon("dialog-information-symbolic")
def _(ic):
    ic.ring(8, 8, 6).dot(8, 4.9, 1).seg((8, 7.4), (8, 11.2), 1.6)


@icon("dialog-warning-symbolic")
def _(ic):
    ic.line([(8, 1.8), (14.6, 13.6), (1.4, 13.6)], close=True).seg((8, 6), (8, 9.4), 1.6).dot(8, 11.5, 0.95)


@icon("dialog-password-symbolic")
def _(ic):
    ic.ring(5, 8, 2.8).seg((7.8, 8), (14.5, 8)).seg((12.5, 8), (12.5, 10.6)).seg((14.5, 8), (14.5, 10.2))


@icon("document-open-recent-symbolic")
def _(ic):
    clock(ic)


@icon("preferences-system-time-symbolic")
def _(ic):
    clock(ic)


@icon("document-open-symbolic")
def _(ic):
    folder(ic)
    ic.seg((8, 11.4), (8, 7.4)); arrowhead(ic, (8, 7.2), -90, 2.2)


@icon("edit-find-symbolic")
def _(ic):
    ic.ring(6.6, 6.6, 4.4).seg((9.9, 9.9), (14, 14), 1.9)


@icon("system-search-symbolic")
def _(ic):
    ic.ring(6.6, 6.6, 4.4).seg((9.9, 9.9), (14, 14), 1.9)


@icon("edit-undo-symbolic")
def _(ic):
    # A hook: down the right side and round the bottom of a half circle, then left along the top to the arrowhead.
    ic.arc(9.2, 9.4, 3.6, -90, 90).seg((9.2, 13), (6.8, 13)).seg((9.2, 5.8), (3.6, 5.8))
    arrowhead(ic, (3.2, 5.8), 180, 2.6)


@icon("emblem-synchronizing-symbolic")
def _(ic):
    ic.arc(8, 8, 5.3, 200, 330).arc(8, 8, 5.3, 20, 150)
    arrowhead(ic, (12.6, 5.4), 70, 2.2); arrowhead(ic, (3.4, 10.6), 250, 2.2)


@icon("emblem-system-symbolic")
def _(ic):
    ic.ring(8, 8, 3.4, 2.2)
    for k in range(8):
        a = math.radians(k * 45)
        ic.seg((8 + 4.4 * math.cos(a), 8 + 4.4 * math.sin(a)), (8 + 6.3 * math.cos(a), 8 + 6.3 * math.sin(a)), 2.4, cap=False)


@icon("folder-symbolic")
def _(ic):
    folder(ic)


@icon("folder-download-symbolic")
def _(ic):
    folder(ic)
    ic.seg((8, 7.2), (8, 11)); arrowhead(ic, (8, 11.2), 90, 2.2)


@icon("folder-music-symbolic")
def _(ic):
    folder(ic)
    ic.dot(7.2, 10.8, 1.3).seg((8.3, 10.8), (8.3, 7.4), 1.2).seg((8.3, 7.4), (10.2, 8.2), 1.2)


for nm, ang in [("go-down-symbolic", 90), ("go-up-symbolic", -90), ("go-next-symbolic", 0), ("go-previous-symbolic", 180)]:
    ic = Icon()
    a = math.radians(ang)
    tip = (8 + 2.2 * math.cos(a), 8 + 2.2 * math.sin(a))
    arrowhead(ic, tip, ang, 5.2, 1.8)
    ICONS[nm] = ic


@icon("go-home-symbolic")
def _(ic):
    ic.line([(1.8, 8.2), (8, 2.4), (14.2, 8.2)]).line([(3.6, 7), (3.6, 14), (12.4, 14), (12.4, 7)]).rect(6.6, 9.6, 2.8, 4.4)


@icon("input-keyboard-symbolic")
def _(ic):
    ic.line([(1.2, 3.8), (14.8, 3.8), (14.8, 12.2), (1.2, 12.2)], close=True)
    for x in (4, 6.7, 9.3, 12):
        ic.rect(x - 0.7, 5.9, 1.4, 1.4)
    ic.rect(4.6, 9, 6.8, 1.4)


@icon("list-add-symbolic")
def _(ic):
    ic.seg((8, 2.6), (8, 13.4), 1.8).seg((2.6, 8), (13.4, 8), 1.8)


@icon("list-remove-symbolic")
def _(ic):
    ic.seg((2.6, 8), (13.4, 8), 1.8)


@icon("media-optical-symbolic")
def _(ic):
    ic.ring(8, 8, 6).ring(8, 8, 1.6, 1.2)


@icon("media-playback-start-symbolic")
def _(ic):
    ic.poly([(4, 2.4), (13.6, 8), (4, 13.6)])


@icon("media-playback-pause-symbolic")
def _(ic):
    ic.rect(3.5, 2.5, 3.2, 11).rect(9.3, 2.5, 3.2, 11)


@icon("media-skip-forward-symbolic")
def _(ic):
    ic.poly([(2, 3), (10, 8), (2, 13)]).rect(11.2, 3, 2.6, 10)


@icon("media-skip-backward-symbolic")
def _(ic):
    ic.poly([(14, 3), (6, 8), (14, 13)]).rect(2.2, 3, 2.6, 10)


def repeat(ic):
    ic.line([(2.2, 9), (2.2, 6.2), (3.6, 4.8), (12.6, 4.8)]).line([(13.8, 7), (13.8, 9.8), (12.4, 11.2), (3.4, 11.2)])
    arrowhead(ic, (13.2, 4.8), 0, 2.2); arrowhead(ic, (2.8, 11.2), 180, 2.2)


@icon("media-playlist-repeat-symbolic")
def _(ic):
    repeat(ic)


@icon("media-playlist-repeat-song-symbolic")
def _(ic):
    repeat(ic)
    ic.seg((8, 6.6), (8, 9.6), 1.4).seg((8, 6.6), (6.9, 7.4), 1.2)


@icon("media-playlist-shuffle-symbolic")
def _(ic):
    ic.line([(1.6, 4.4), (4.4, 4.4), (10.6, 11.6), (13.2, 11.6)]).line([(1.6, 11.6), (4.4, 11.6), (6.6, 9)]).line([(9.2, 6.6), (10.6, 4.4), (13.2, 4.4)])
    arrowhead(ic, (13.8, 4.4), 0, 2.2); arrowhead(ic, (13.8, 11.6), 0, 2.2)


@icon("multimedia-equalizer-symbolic")
def _(ic):
    for x, k in ((3.5, 10), (8, 5), (12.5, 8.5)):
        ic.seg((x, 2), (x, 14), 1.2).rect(x - 2, k - 1.1, 4, 2.2)


@icon("network-offline-symbolic")
def _(ic):
    ic.arc(8, 13, 9.5, 225, 315).arc(8, 13, 6.2, 225, 315).dot(8, 12.6, 1.4).seg((2.4, 2.4), (13.6, 13.6), 1.6)


@icon("object-select-symbolic")
def _(ic):
    ic.line([(2.6, 8.4), (6.2, 12), (13.4, 4.4)], 1.9)


@icon("preferences-desktop-appearance-symbolic")
def _(ic):
    ic.ring(8, 8, 6)
    ic.poly([(8, 2)] + [(8 + 6 * math.cos(math.radians(a)), 8 + 6 * math.sin(math.radians(a))) for a in range(-90, 91, 10)])


@icon("view-fullscreen-symbolic")
def _(ic):
    for sx, sy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        c = (8 - sx * 5.8, 8 - sy * 5.8)
        ic.line([(c[0] + sx * 3.4, c[1]), c, (c[0], c[1] + sy * 3.4)])


@icon("view-restore-symbolic")
def _(ic):
    for sx, sy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        c = (8 - sx * 2.4, 8 - sy * 2.4)
        ic.line([(c[0] - sx * 3.4, c[1]), c, (c[0], c[1] - sy * 3.4)])


@icon("view-list-symbolic")
def _(ic):
    for y in (4, 8, 12):
        ic.dot(2.6, y, 1).seg((5.6, y), (14, y))


@icon("view-list-ordered-symbolic")
def _(ic):
    for y in (4, 8, 12):
        ic.seg((6, y), (14, y))
    ic.seg((2.6, 2.6), (2.6, 5.4), 1.2).line([(1.8, 7), (3.4, 7), (1.8, 9.2), (3.4, 9.2)], 1.1).line([(1.8, 10.8), (3.4, 10.8), (2.4, 12), (3.4, 13.2), (1.8, 13.2)], 1.1)


@icon("view-more-symbolic")
def _(ic):
    ic.dot(3, 8, 1.5).dot(8, 8, 1.5).dot(13, 8, 1.5)


@icon("view-sort-descending-symbolic")
def _(ic):
    ic.seg((1.8, 3.5), (10, 3.5)).seg((1.8, 8), (7.6, 8)).seg((1.8, 12.5), (5.2, 12.5))
    ic.seg((12.8, 3), (12.8, 12.6)); arrowhead(ic, (12.8, 13), 90, 2.4)


@icon("weather-clear-night-symbolic")
def _(ic):
    # A crescent, outlined directly (a subtracted disc would fill wherever it pokes outside under the nonzero rule): the
    # outer circle's arc outside the inner circle, then the inner circle's arc inside the outer one, back to the start.
    (ox, oy, orr), (ix, iy, ir) = (7.4, 8.6, 5.8), (10.6, 5.6, 4.9)
    inside = lambda x, y, cx, cy, r: (x - cx) ** 2 + (y - cy) ** 2 < r * r
    outer = [p for p in circle_pts(ox, oy, orr, 240) if not inside(*p, ix, iy, ir)]
    inner = [p for p in circle_pts(ix, iy, ir, 240) if inside(*p, ox, oy, orr)]
    # Rotate each list so it runs as one contiguous arc (the sample order wraps at angle 0).
    def contiguous(pts):
        gaps = [math.dist(pts[i], pts[(i + 1) % len(pts)]) for i in range(len(pts))]
        k = max(range(len(pts)), key=lambda i: gaps[i])
        return pts[k + 1:] + pts[:k + 1]
    outer, inner = contiguous(outer), contiguous(inner)
    if math.dist(outer[-1], inner[0]) > math.dist(outer[-1], inner[-1]):
        inner = inner[::-1]
    ic.poly(outer + inner)


@icon("window-new-symbolic")
def _(ic):
    ic.line([(1.6, 3), (14.4, 3), (14.4, 13), (1.6, 13)], close=True).rect(8.2, 7.8, 4.6, 3.6)


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    for name, ic in sorted(ICONS.items()):
        with open(os.path.join(here, f"{name}.svg"), "w") as f:
            f.write(ic.svg())
    # The list icons.rs embeds (include!), so adding an icon here is all it takes.
    with open(os.path.join(here, "list.rs"), "w") as f:
        f.write("// Generated by make.py: the fallback icons as (file name, SVG).\n&[\n")
        for name in sorted(ICONS):
            f.write(f'    ("{name}.svg", include_str!("{name}.svg")),\n')
        f.write("]\n")
    print(f"wrote {len(ICONS)} icons")
