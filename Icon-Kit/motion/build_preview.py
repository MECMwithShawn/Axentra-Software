"""Build a portable preview with embedded canonical PNGs; Python standard library only."""
from pathlib import Path
import argparse
import base64
import json

ROOT = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=ROOT.parent / 'out/axentra-motion-preview.html')
args = parser.parse_args()
source = (ROOT / 'index.html').read_text(encoding='utf-8')
apps = json.loads((ROOT.parent / 'build-info.json').read_text(encoding='utf-8'))['app_order']
images = {app: 'data:image/png;base64,' + base64.b64encode((ROOT.parent / f'assets/{app}-256.png').read_bytes()).decode()
          for app in apps}
source = source.replace('<link rel="stylesheet" href="motion.css">', '<style>'+(ROOT / 'motion.css').read_text(encoding='utf-8')+'</style>')
source = source.replace('<script src="axentra-motion.js"></script>', '<script>window.AXENTRA_EMBEDDED_ICONS='+json.dumps(images)+';</script>\n<script>'+(ROOT / 'axentra-motion.js').read_text(encoding='utf-8')+'</script>')
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(source, encoding='utf-8', newline='\n')
print(args.output.resolve())
