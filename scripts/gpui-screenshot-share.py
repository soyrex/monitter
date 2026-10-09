#!/usr/bin/env python3
"""Encode an existing screenshot for phone review; never captures or controls UI.

Keeps the full-resolution source untouched. The JPEG and its base64 payload are
both bounded before delivery, including headroom below the 512 KiB transport cap.
"""
import argparse
import base64
import io
import json
from pathlib import Path
from PIL import Image, ImageOps

RAW_LIMIT = 300 * 1024
TRANSPORT_LIMIT = 450 * 1024


def encode(source: Path, output: Path, crop=None, max_edge=1600):
    with Image.open(source) as original:
        original.load()
        image = ImageOps.exif_transpose(original).convert('RGB')
    source_size = image.size
    if crop:
        left, top, right, bottom = crop
        if not (0 <= left < right <= image.width and 0 <= top < bottom <= image.height):
            raise ValueError('Crop must stay within the source screenshot.')
        image = image.crop(crop)
    for edge in (max_edge, min(max_edge, 1440), min(max_edge, 1200), min(max_edge, 960)):
        resized = image.copy()
        resized.thumbnail((edge, edge), Image.Resampling.LANCZOS)
        for quality in (88, 82, 76, 68):
            buffer = io.BytesIO()
            resized.save(buffer, format='JPEG', quality=quality, optimize=True)
            data = buffer.getvalue()
            transport_bytes = len(base64.b64encode(data)) + 1024
            if len(data) <= RAW_LIMIT and transport_bytes <= TRANSPORT_LIMIT:
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_bytes(data)
                result = {'source': str(source), 'output': str(output),
                          'sourceDimensions': list(source_size), 'dimensions': list(resized.size),
                          'crop': list(crop) if crop else None, 'quality': quality,
                          'imageBytes': len(data), 'estimatedInlineBytes': transport_bytes,
                          'imageLimitBytes': RAW_LIMIT, 'inlineLimitBytes': TRANSPORT_LIMIT}
                output.with_suffix(output.suffix + '.json').write_text(json.dumps(result, indent=2) + '\n')
                return result
    raise ValueError('Screenshot could not fit the delivery limits; use a smaller focused crop.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--crop', type=int, nargs=4, metavar=('LEFT', 'TOP', 'RIGHT', 'BOTTOM'))
    parser.add_argument('--max-edge', type=int, default=1600)
    args = parser.parse_args()
    if args.max_edge < 320 or args.max_edge > 2560:
        parser.error('--max-edge must be between 320 and 2560.')
    print(json.dumps(encode(args.source, args.output, args.crop, args.max_edge)))
