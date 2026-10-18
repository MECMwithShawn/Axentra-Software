"""Copy approved icon assets into existing sibling projects. Default: print plan only."""
import argparse
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
PROJECTS = {
    "command": "axentra-command", "ghost": "axentra-ghost",
    "vault": "axentra-vault", "crystalwire": "axentra-crystalwire",
    "horizon": "stillpoint", "pdf": "REA/axentra-pdf", "spaces": "axentra-spaces",
}


def mapping(app):
    pairs = []
    def add(source, target):
        pairs.append((ROOT / source, Path(PROJECTS[app]) / target))
    if app == "command":
        add("assets/command.ico", "assets/command.ico")
        for app_id in ("horizon", "crystalwire"):
            add(f"svg/{app_id}.svg", f"assets/{app_id}-outline.svg")
        for name in ("Cargo.toml", "Cargo.lock", "src/main.rs", "src/suite.rs"):
            add(f"renderer/{name}", f"tools/suite-icons/{name}")
        for path in sorted((ROOT / "assets").iterdir()):
            add(f"assets/{path.name}", f"tools/suite-icons/out/{path.name}")
    elif app == "ghost":
        add("assets/ghost.ico", "assets/ghost.ico")
        add("renderer/src/suite.rs", "src/suite_icon.rs")
    elif app == "vault":
        add("assets/vault.ico", "app/assets/app/vault.ico")
        for size in SIZES:
            add(f"assets/vault-{size}.png", f"app/assets/app/vault-{size}.png")
    elif app == "crystalwire":
        add("svg/crystalwire.svg", "assets/crystalwire.svg")
        add("assets/crystalwire.ico", "assets/crystalwire.ico")
        for size in SIZES:
            add(f"assets/crystalwire-{size}.png", f"assets/crystalwire-{size}.png")
    elif app == "horizon":
        add("svg/horizon.svg", "icons/horizon.svg")
        add("svg/horizon-16.svg", "icons/horizon-16.svg")
        add("assets/horizon.ico", "icons/horizon.ico")
        add("assets/horizon-128.png", "icons/apps/horizon.png")
        for size in SIZES:
            add(f"assets/horizon-{size}.png", f"icons/{size}.png")
    elif app == "pdf":
        add("assets/pdf.ico", "packaging/icons/axentra.ico")
        for size in SIZES:
            add(f"assets/pdf-{size}.png", f"packaging/icons/axentra-{size}.png")
            if size <= 256:
                add(f"assets/pdf-{size}.png", f"axentra/ui/brand/axentra-{size}.png")
    elif app == "spaces":
        add("assets/spaces.ico", "src/AxentraSpaces/Assets/spaces.ico")
        add("assets/spaces-512.png", "spaces-512.png")
    return pairs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--code-root", required=True, type=Path, help="Parent of the existing application checkouts")
    parser.add_argument("--apps", nargs="+", choices=PROJECTS, default=list(PROJECTS))
    parser.add_argument("--apply", action="store_true", help="Copy the displayed files; review git status first")
    args = parser.parse_args()
    code_root = args.code_root.resolve(strict=True)
    selected = list(dict.fromkeys(args.apps))
    plan = []
    for app in selected:
        project = (code_root / PROJECTS[app]).resolve()
        if not project.is_dir() or not project.is_relative_to(code_root):
            raise SystemExit(f"Missing project or path outside code root: {project}")
        for source, relative_target in mapping(app):
            target = (code_root / relative_target).resolve()
            if not source.is_file() or not target.is_relative_to(project):
                raise SystemExit(f"Invalid copy: {source} -> {target}")
            plan.append((source, target))
    preview = None
    if "pdf" in selected:
        preview = (code_root / PROJECTS["pdf"] / "packaging/windows/preview-app.bmp").resolve()
        if not preview.is_relative_to(code_root / PROJECTS["pdf"]):
            raise SystemExit("PDF preview resolves outside the project")
    # Resolve the only optional dependency before the first write.
    if args.apply and preview:
        from PIL import Image
    for source, target in plan:
        print(f"{source.relative_to(ROOT)} -> {target}")
    if preview:
        print(f"PDF 96 px installer preview -> {preview}")
    if not args.apply:
        print(f"Dry run: {len(plan)} copies. Add --apply after reviewing the plan.")
        return
    for source, target in plan:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    if preview:
        preview.parent.mkdir(parents=True, exist_ok=True)
        with Image.open(ROOT / "assets/pdf-256.png") as image:
            tile = Image.alpha_composite(Image.new("RGBA", image.size, "white"), image.convert("RGBA"))
            tile.convert("RGB").resize((96, 96), Image.Resampling.LANCZOS).save(preview)
    print(f"Copied {len(plan)} files. Rebuild the selected applications. PDF document icons were not modified.")


if __name__ == "__main__":
    main()
