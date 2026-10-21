#!/usr/bin/env python3
"""Move Trunk's generated inline bootstrap to a same-origin file for strict CSP."""
from pathlib import Path
import re
import sys

root = Path(sys.argv[1] if len(sys.argv) > 1 else 'dist')
page = root / 'index.html'
html = page.read_text()
pattern = re.compile(r'<script type="module">(.*?)</script>', re.S)
matches = list(pattern.finditer(html))
if len(matches) != 1:
    raise SystemExit(f'Expected one generated Trunk module, found {len(matches)}')
(root / 'bootstrap.js').write_text(matches[0].group(1))
page.write_text(pattern.sub('<script type="module" src="bootstrap.js"></script>', html))
