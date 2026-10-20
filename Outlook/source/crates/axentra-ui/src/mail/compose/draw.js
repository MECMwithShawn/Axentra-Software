// Drawing canvas (Draw tab), run inside `document::eval`.
//
// Rust embeds {id} where the placeholder is. Strokes are kept as point
// lists so Undo can redraw; the canvas itself stays transparent and the
// exported picture is flattened onto white. Reports `{n, all}` (ink and
// all strokes) through `dioxus.send`; Rust drives it through `window.axDraw[id]`:
// `.set({tool, color, width})`, `.undo()`, `.clear()`, `.data()`.
const cfg = /*AX_CONFIG*/null;
let canvas = null;
for (let i = 0; i < 100 && !canvas; i++) {
  canvas = document.getElementById(cfg.id);
  if (!canvas) await new Promise((r) => setTimeout(r, 20));
}
if (!canvas) return;
const dpr = window.devicePixelRatio || 1;
const ctx = canvas.getContext("2d");
let cssW = 0, cssH = 0;
// The bitmap matches the canvas's CSS size (the dialog is resizable).
function fitBitmap() {
  cssW = canvas.clientWidth;
  cssH = canvas.clientHeight;
  canvas.width = Math.round(cssW * dpr);
  canvas.height = Math.round(cssH * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}
fitBitmap();
const pen = { tool: "pen", color: "#000000", width: 4 };
const strokes = [];
let current = null;

function styleFor(s) {
  ctx.globalCompositeOperation = s.tool === "eraser" ? "destination-out" : "source-over";
  ctx.globalAlpha = s.tool === "highlighter" ? 0.4 : 1;
  ctx.strokeStyle = s.tool === "eraser" ? "#000" : s.color;
  ctx.lineWidth = s.tool === "highlighter" ? s.width * 3 : s.tool === "eraser" ? s.width * 4 : s.width;
  ctx.lineCap = s.tool === "highlighter" ? "square" : "round";
  ctx.lineJoin = "round";
}
function drawStroke(s) {
  styleFor(s);
  ctx.beginPath();
  s.points.forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
  if (s.points.length === 1) ctx.lineTo(s.points[0][0] + 0.01, s.points[0][1]);
  ctx.stroke();
}
function redraw() {
  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.restore();
  strokes.forEach(drawStroke);
}
const report = () => dioxus.send({ n: strokes.filter((s) => s.tool !== "eraser").length, all: strokes.length });
function point(e) {
  const r = canvas.getBoundingClientRect();
  return [e.clientX - r.left, e.clientY - r.top];
}
canvas.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  e.preventDefault();
  canvas.setPointerCapture(e.pointerId);
  current = { ...pen, points: [point(e)] };
  strokes.push(current);
  drawStroke(current);
});
canvas.addEventListener("pointermove", (e) => {
  if (!current) return;
  current.points.push(point(e));
  // Redraw the whole stroke: overlapping segments would darken highlighter ink.
  redraw();
});
const end = () => { if (current) { current = null; report(); } };
// A resized dialog resizes the paper: strokes keep their place from the
// top-left corner and are redrawn at the new size.
const resized = window.ResizeObserver
  ? new ResizeObserver(() => {
      if (canvas.clientWidth && canvas.clientHeight && (canvas.clientWidth !== cssW || canvas.clientHeight !== cssH)) {
        fitBitmap();
        redraw();
      }
    })
  : null;
if (resized) resized.observe(canvas);
canvas.addEventListener("pointerup", end);
canvas.addEventListener("pointercancel", end);

const api = {
  set: (v) => Object.assign(pen, v),
  undo: () => { strokes.pop(); redraw(); report(); },
  clear: () => { strokes.length = 0; redraw(); report(); },
  data: () => {
    if (!strokes.some((s) => s.tool !== "eraser")) return null;
    const out = document.createElement("canvas");
    out.width = canvas.width;
    out.height = canvas.height;
    const o = out.getContext("2d");
    o.fillStyle = "#ffffff";
    o.fillRect(0, 0, out.width, out.height);
    o.drawImage(canvas, 0, 0);
    return { src: out.toDataURL("image/png"), width: cssW, height: cssH };
  },
};
window.axDraw = window.axDraw || {};
window.axDraw[cfg.id] = api;
report();
await new Promise((resolve) => {
  const timer = setInterval(() => {
    if (!canvas.isConnected) {
      clearInterval(timer);
      if (resized) resized.disconnect();
      if (window.axDraw[cfg.id] === api) delete window.axDraw[cfg.id];
      resolve();
    }
  }, 1000);
});
