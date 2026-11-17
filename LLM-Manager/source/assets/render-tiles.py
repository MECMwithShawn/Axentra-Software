#!/usr/bin/env python3
"""Renders the in-app state tiles as raw RGBA (straight alpha) for include_bytes!.

The app icon is not rendered here: it is assets/llm-manager-256.png (master: llm-manager.svg).

The tile recipe (glass tile, rim, glow, sheen, state badge) follows Lab Manager's
STATE_SVG so both apps read as one family. Run: python3 assets/render-tiles.py
"""
import os
import gi
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import GdkPixbuf

HERE = os.path.dirname(os.path.realpath(__file__))

GLYPHS = {
    'server': '''<rect x="11" y="9" width="18" height="9" rx="2" fill="none" stroke="{fg}" stroke-width="1.8"/>
<rect x="11" y="22" width="18" height="9" rx="2" fill="none" stroke="{fg}" stroke-width="1.8"/>
<circle cx="15" cy="13.5" r="1.3" fill="{fg}"/><circle cx="15" cy="26.5" r="1.3" fill="{fg}"/>
<path d="M19 13.5h6M19 26.5h6" stroke="{fg}" stroke-width="1.6" stroke-linecap="round"/>''',
}

STATE_SVG = '''<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 40 40">
<defs>
  <linearGradient id="bg" x1="0" y1="0" x2="0.35" y2="1">
    <stop offset="0" stop-color="{bg0}" stop-opacity="{bgo0}"/>
    <stop offset="1" stop-color="{bg1}" stop-opacity="{bgo1}"/>
  </linearGradient>
  <linearGradient id="rim" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="{rim0}" stop-opacity="{rimo0}"/>
    <stop offset="1" stop-color="{rim1}" stop-opacity="{rimo1}"/>
  </linearGradient>
  <linearGradient id="glyph" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{g0}"/><stop offset="1" stop-color="{g1}"/>
  </linearGradient>
  <radialGradient id="glow" cx="0.5" cy="0.45" r="0.6">
    <stop offset="0" stop-color="{glow}" stop-opacity="{glowo}"/><stop offset="1" stop-color="{glow}" stop-opacity="0"/>
  </radialGradient>
  <radialGradient id="badge" cx="0.35" cy="0.3" r="0.8">
    <stop offset="0" stop-color="{b0}"/><stop offset="1" stop-color="{b1}"/>
  </radialGradient>
</defs>
<rect x="0.5" y="0.5" width="39" height="39" rx="9" fill="url(#bg)" stroke="url(#rim)" stroke-width="1"/>
<rect x="0.5" y="0.5" width="39" height="39" rx="9" fill="url(#glow)"/>
<path d="M3 13 Q4 4 13 3 L19 3 Q8 6 5 17 Z" fill="#ffffff" fill-opacity="{sheen}"/>
{glyph}
{badge_svg}
</svg>'''

BADGE = '''<circle cx="32" cy="32" r="6.2" fill="url(#badge)" stroke="{bstroke}" stroke-width="0.9"/>
<g transform="translate(32 32)">{badge}</g>'''

RUN_BADGE = ('<circle r="4.3" fill="none" stroke="#ffffff" stroke-opacity="0.75" stroke-width="0.9"/>'
             '<circle r="2.4" fill="#ffffff"/>')
OFF_BADGE = ('<path d="M-2.6 -2.2 A3.5 3.5 0 1 0 2.6 -2.2" fill="none" stroke="#d9dee4" stroke-width="1.35" '
             'stroke-linecap="round"/><path d="M0 -4.2 V-0.4" stroke="#d9dee4" stroke-width="1.35" stroke-linecap="round"/>')
BUSY_BADGE = ''.join(f'<circle cx="{x}" r="1.05" fill="#5a3c00"/>' for x in (-2.8, 0, 2.8))

LIT = dict(bg0='#2fa8ff', bgo0=0.42, bg1='#0a3f8f', bgo1=0.62, rim0='#8fdcff', rimo0=0.9,
           rim1='#1f86e8', rimo1=0.8, g0='#ffffff', g1='#7fd8ff', glow='#3fb8ff', glowo=0.5, sheen=0.14)
STYLES = {
    'loaded': dict(LIT, b0='#7ff0a0', b1='#1fae55', bstroke='#0f7a3a', badge=RUN_BADGE),
    'busy': dict(LIT, b0='#ffe7a3', b1='#e2a93a', bstroke='#9a6a10', badge=BUSY_BADGE),
    'idle': dict(bg0='#9aa6b2', bgo0=0.14, bg1='#3a4048', bgo1=0.30, rim0='#ffffff', rimo0=0.22,
                 rim1='#ffffff', rimo1=0.08, g0='#a9b1ba', g1='#6f7882', glow='#000000', glowo=0.0,
                 sheen=0.06, b0='#6c747d', b1='#3d434a', bstroke='#2a2f35', badge=OFF_BADGE),
    'gpu': dict(LIT, b0='#000000', b1='#000000', bstroke='#000000', badge=None),
}

def write(name, svg, size):
    loader = GdkPixbuf.PixbufLoader.new_with_type('svg')
    loader.write(svg.encode())
    loader.close()
    pix = loader.get_pixbuf()
    assert pix.get_width() == size and pix.get_height() == size and pix.get_has_alpha()
    stride, raw = pix.get_rowstride(), pix.get_pixels()
    rows = b''.join(raw[y * stride:y * stride + size * 4] for y in range(size))
    with open(os.path.join(HERE, name + '.rgba'), 'wb') as out:
        out.write(rows)
    pix.savev(os.path.join(HERE, name + '.png'), 'png', [], [])


# The GPU heading keeps the rack glyph; the processor is the app icon's own symbol.
for key, glyph in (('loaded', 'server'), ('busy', 'server'), ('idle', 'server'), ('gpu', 'server')):
    style = dict(STYLES[key])
    badge = style.pop('badge')
    bstroke = style.pop('bstroke')
    badge_svg = BADGE.format(badge=badge, bstroke=bstroke) if badge else ''
    write('tile-' + key, STATE_SVG.format(size=96, glyph=GLYPHS[glyph].format(fg='url(#glyph)'),
                                          badge_svg=badge_svg, **style), 96)
