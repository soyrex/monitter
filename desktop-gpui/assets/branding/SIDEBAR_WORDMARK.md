# Sidebar wordmark raster sizes

The unchanged `monitter-wordmark.png` is the 2169 × 725 RGBA master supplied
for Monitter. `monitter-wordmark-{width}.png` variants at 96, 144, 192, 240,
288, 336 and 384 pixels wide are premultiplied-alpha Lanczos reductions of that
master, converted back to straight RGBA for PNG storage. No sharpening or
redrawing is applied.

`branding::sidebar_wordmark` chooses the smallest variant covering the rendered
width × interface scale × display scale. The sidebar's optical box remains
91.2 × 30.4 design pixels, centered within its header; the variants prevent
aliasing from the atlas's large single-step bilinear reduction.
