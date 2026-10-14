# Reusable Prompt: Professional Application Icon Kit Generator

Create **two complete, production-ready icon kits** for **[APPLICATION NAME]**, using the application's existing branding, color palette and any supplied reference images.

## 1. Icon Design Variants

**Kit 1: Flat Windows Style**
- Clean, minimalist design optimized for Windows 10/11
- Flat colors, sharp geometry and consistent proportions
- High visibility in File Explorer, taskbar and desktop shortcuts
- Simplified details for smaller icon sizes

**Kit 2: Premium Glossy Style**
- Modern, premium appearance with refined gradients
- Subtle glass effects, highlights, depth and soft shadows
- Consistent with the application's visual identity
- Optimized for readability at all Windows icon sizes

## 2. Required Deliverables

Generate actual, usable icon assets for **both kits**, not just concept sheets or mockups.

Each kit must contain:

**Editable SVG Masters**
- Primary icon
- Light and dark variants
- Monochrome variant
- Simplified small-size variant
- Proper vector paths and scalable geometry

**Windows ICO Files**
- Multi-resolution `.ico` containing 16×16, 24×24, 32×32, 48×48, 64×64, 128×128 and 256×256 resolutions
- Compatible with Windows EXE, MSI, shortcuts and file associations

**PNG Exports**
- Transparent PNG images
- Sizes: 16×16, 24×24, 32×32, 48×48, 64×64, 128×128, 256×256, 512×512 and 1024×1024
- Light, dark and monochrome variations where appropriate

**Preview Sheet**
- Side-by-side icon comparisons
- Light and dark background examples
- All icon sizes
- Windows File Explorer and taskbar previews

## 3. Packaging Structure

Package the deliverables as:

```text
[Application]_Icon_Kits/
│
├── Flat_Windows/
│   ├── SVG/
│   ├── ICO/
│   ├── PNG/
│   └── Preview.png
│
├── Premium_Glossy/
│   ├── SVG/
│   ├── ICO/
│   ├── PNG/
│   └── Preview.png
│
├── Installer_Integration/
│   ├── Registry/
│   ├── WiX/
│   └── Inno_Setup/
│
└── README.md
```

## 4. Installer Integration

Include functional examples for:

- Windows Registry (`.reg`)
- WiX Toolset
- Inno Setup
- EXE application icon
- MSI installer branding
- Desktop and Start menu shortcuts
- File association icons where applicable

Use an application-specific ProgID for file associations. Do not forcibly overwrite the user's existing Windows default application.

## 5. Quality Validation

Before delivery:

- Verify SVG files are valid and render correctly.
- Verify ICO files contain all required resolutions.
- Check PNG dimensions, transparency and visual quality.
- Optimize small icons independently when necessary.
- Ensure the flat and glossy variants are visually distinct.
- Confirm all referenced assets exist in the ZIP archives.
- Ensure installer examples reference the correct filenames.
- Preserve brand consistency across every variation.

## 6. Final Downloads

Provide three downloadable ZIP files:

1. **[Application]_Flat_Windows_Icon_Kit.zip**
2. **[Application]_Premium_Glossy_Icon_Kit.zip**
3. **[Application]_Complete_Icon_Kits.zip**

The combined ZIP must include both complete icon kits, preview sheets, source vectors and installer integration examples.

**Critical requirement:** Render the designs for visual review, but also generate the actual SVG, ICO and PNG assets. A preview sheet alone is not an acceptable deliverable.

Deliver production-ready assets that developers can immediately integrate into their Windows application and installer without manually extracting artwork from images.
