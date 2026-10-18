"""Export the full current icon family: Rust baseline followed by approved SVG overrides."""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", nargs="?", type=Path, default=ROOT / "out")
    args = parser.parse_args()
    output = args.output.resolve()
    # Generation must never overwrite the approved assets or source tree.
    if output == ROOT or output in ROOT.parents or any(
        output == ROOT / folder or (ROOT / folder) in output.parents
        for folder in ("assets", "svg", "renderer", "references", "document-icons", "scripts", "motion")
    ):
        parser.error("Choose a separate output folder, such as Icon-Kit/out")
    subprocess.run(["node", str(ROOT / "svg/render.cjs"), "--versions"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["cargo", "run", "--manifest-path", str(ROOT / "renderer/Cargo.toml"),
                    "--release", "--locked", "--offline", "--", str(output)], check=True)
    subprocess.run(["node", str(ROOT / "svg/render.cjs"), str(output)], check=True)


if __name__ == "__main__":
    main()
