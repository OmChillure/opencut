#!/usr/bin/env node
// Render one catalog design to an mp4. Chrome seeks the page; ffmpeg encodes the frames.

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const FPS = 24;

// A comma is part of the number. "10,000 prize pool" is 10000 plus its label.
export function readFigure(text) {
  const m = String(text || "").match(/-?\$?\d[\d,]*(?:\.\d+)?%?/);
  if (!m) return null;
  const raw = m[0];
  const n = Number(raw.replace(/[$,%]/g, "").replace(/,/g, ""));
  if (!Number.isFinite(n)) return null;
  const frac = raw.split(".")[1] || "";
  return {
    n,
    raw,
    prefix: raw.includes("$") ? "$" : "",
    suffix: raw.includes("%") ? "%" : "",
    decimals: frac.replace(/\D/g, "").length,
  };
}

function stripFigure(text, raw) {
  if (!text || !raw) return "";
  return String(text).split(raw).join(" ").replace(/[$%]/g, " ").replace(/\s+/g, " ").trim();
}

export function bandCopy(text, prompt) {
  const figure = readFigure(text) || readFigure(prompt);
  if (!figure) {
    const line = String(text || "").trim() || String(prompt || "").trim();
    return { figure: null, label: "", line };
  }
  const label = stripFigure(text, figure.raw) || stripFigure(prompt, figure.raw);
  return { figure, label, line: "" };
}

const STYLES = {
  bold: { bg: "#111111", fg: "#f4f1ea", accent: "#ff4d2e", muted: "#8d867c", serif: false },
  editorial: { bg: "#1a1814", fg: "#f3efe6", accent: "#c4a574", muted: "#9a907f", serif: true },
  swiss: { bg: "#0e0e0e", fg: "#ffffff", accent: "#e10600", muted: "#b5b5b5", serif: false },
  terminal: { bg: "#0c1210", fg: "#d6ffe8", accent: "#3dff9a", muted: "#6f8f7c", serif: false },
  spotlight: { bg: "#120c18", fg: "#ffffff", accent: "#b388ff", muted: "#9a8aab", serif: false },
  clean: { bg: "#f4f7fb", fg: "#102033", accent: "#2f6fed", muted: "#5c6b7a", serif: false },
  neon: { bg: "#07070c", fg: "#f4f4f4", accent: "#3df0ff", muted: "#7d8b96", serif: false },
};

function arg(name, fallback = "") {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

function page(spec) {
  const style = STYLES[spec.style] || STYLES.bold;
  const band = spec.h < spec.w * 0.5;
  const copy = bandCopy(spec.text, spec.prompt);
  const view = {
    ...spec,
    figure: copy.figure ? copy.figure.n : null,
    prefix: copy.figure ? copy.figure.prefix : "",
    suffix: copy.figure ? copy.figure.suffix : "",
    decimals: copy.figure ? copy.figure.decimals : 0,
    label: copy.label,
    line: copy.line,
  };
  const font = !band && style.serif
    ? '"Liberation Serif", "Times New Roman", serif'
    : '"Liberation Sans", "DejaVu Sans", sans-serif';
  const ground = band ? "transparent" : style.bg;
  const data = JSON.stringify(view).replace(/</g, "\\u003c");
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
  html, body { margin:0; width:${spec.w}px; height:${spec.h}px; overflow:hidden; background:${ground}; color:${style.fg}; font-family:${font}; }
  #root { position:relative; width:100%; height:100%; background:transparent; }
  .hero { position:absolute; inset:8%; display:flex; align-items:center; justify-content:center; text-align:center; font-weight:760; letter-spacing:-0.04em; line-height:0.92; font-size:${Math.round(spec.w * 0.11)}px; }
  .kicker { position:absolute; left:8%; top:10%; font-size:${Math.round(spec.w * 0.028)}px; letter-spacing:0.16em; text-transform:uppercase; color:${style.muted}; }
  .bar, .call, .quote, .panel, .lock, .map, .chart, .stat { position:absolute; }
  .lockup { position:absolute; inset:0; display:flex; align-items:center; gap:3.5%; padding:0 4% 0 5%; box-sizing:border-box; background:transparent; }
  .rule { width:3px; align-self:stretch; margin:14% 0; background:${style.accent}; border-radius:2px; transform-origin:center center; flex:none; }
  .copy { display:flex; flex-direction:column; justify-content:center; gap:2px; min-width:0; }
  .figure { font-weight:700; letter-spacing:-0.045em; line-height:0.9; font-variant-numeric:tabular-nums; white-space:nowrap; text-shadow:0 1px 2px rgba(0,0,0,.5); }
  .name { font-weight:560; letter-spacing:0.07em; text-transform:uppercase; line-height:1.05; white-space:nowrap; overflow:hidden; text-shadow:0 1px 2px rgba(0,0,0,.55); }
  .line { font-weight:650; letter-spacing:-0.03em; line-height:1.02; text-shadow:0 1px 2px rgba(0,0,0,.55); }
  svg { overflow:visible; }
</style>
</head>
<body>
<div id="root"></div>
<script id="spec" type="application/json">${data}</script>
<script>
const spec = JSON.parse(document.getElementById("spec").textContent);
const root = document.getElementById("root");
const accent = ${JSON.stringify(style.accent)};
const muted = ${JSON.stringify(style.muted)};
const fg = ${JSON.stringify(style.fg)};
const bg = ${JSON.stringify(style.bg)};
function clamp(n, a, b) { return Math.max(a, Math.min(b, n)); }
function ease(p) { p = clamp(p, 0, 1); return 1 - Math.pow(1 - p, 3); }
function back(p) {
  p = clamp(p, 0, 1);
  const c = 1.6;
  return 1 + (c + 1) * Math.pow(p - 1, 3) + c * Math.pow(p - 1, 2);
}
function move(t) {
  const span = Math.min(1.7, spec.dur * 0.62);
  return ease(t / span);
}
function nums() {
  const found = [];
  const re = /-?\\d+(?:\\.\\d+)?/g;
  const blob = spec.prompt + " " + spec.text;
  let m;
  while ((m = re.exec(blob))) found.push(Number(m[0]));
  return found.filter((n) => Number.isFinite(n));
}
function labels() {
  const parts = (spec.prompt || spec.text).split(/[,|/]/).map((s) => s.trim()).filter((s) => /[A-Za-z]/.test(s));
  return parts.length >= 2 ? parts.slice(0, 5) : [];
}
const band = spec.h < spec.w * 0.5;
const design = spec.design;
if (band) buildBand();
else if (design.startsWith("kinetic")) buildKinetic();
else if (design.startsWith("stat")) buildStat();
else if (design.startsWith("chart")) buildChart();
else if (design.startsWith("lower")) buildLower();
else if (design.startsWith("logo")) buildLogo();
else if (design.startsWith("map")) buildMap();
else buildKinetic();

function buildBand() {
  const ink = "#f4f1ea";
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  const lock = document.createElement("div");
  lock.className = "lockup";
  const rule = document.createElement("div");
  rule.className = "rule";
  const copy = document.createElement("div");
  copy.className = "copy";
  lock.appendChild(rule);
  lock.appendChild(copy);
  root.appendChild(lock);
  let numEl = null;
  const figure = spec.figure;
  const decimals = spec.decimals || 0;
  if (figure != null && isFinite(figure)) {
    numEl = document.createElement("div");
    numEl.className = "figure";
    numEl.style.fontSize = Math.round(fitPx(shownFigure(figure, decimals), spec.h * 0.46, spec.w * 0.76)) + "px";
    numEl.style.color = accent;
    copy.appendChild(numEl);
    if (spec.label) {
      const lab = document.createElement("div");
      lab.className = "name";
      lab.textContent = spec.label;
      lab.style.fontSize = Math.round(fitPx(spec.label, spec.h * 0.22, spec.w * 0.76)) + "px";
      lab.style.color = ink;
      copy.appendChild(lab);
    }
  } else {
    const line = document.createElement("div");
    line.className = "line";
    line.textContent = spec.line || spec.text || "";
    const words = line.textContent;
    line.style.fontSize = Math.round(fitPx(words, spec.h * 0.38, spec.w * 0.72)) + "px";
    line.style.color = ink;
    line.style.whiteSpace = words.length > 28 ? "normal" : "nowrap";
    copy.appendChild(line);
  }
  window.seek = (t) => {
    const p = move(t);
    if (numEl) numEl.textContent = shownFigure(figure * p, decimals);
    const rise = (1 - p) * Math.min(10, spec.h * 0.06);
    copy.style.opacity = String(Math.min(1, p * 1.35));
    copy.style.transform = "translateY(" + rise + "px)";
    rule.style.transform = "scaleY(" + Math.max(0.05, p) + ")";
    rule.style.opacity = String(Math.min(1, p * 1.5));
  };
}
function shownFigure(n, decimals) {
  const body = decimals > 0
    ? Number(n).toLocaleString("en-US", { minimumFractionDigits: decimals, maximumFractionDigits: decimals })
    : Math.round(n).toLocaleString("en-US");
  return (spec.prefix || "") + body + (spec.suffix || "");
}
function fitPx(text, maxPx, maxW) {
  const chars = Math.max(1, String(text || "").length);
  const byWidth = maxW / (chars * 0.56);
  return Math.max(14, Math.min(maxPx, byWidth));
}

function buildKinetic() {
  const hero = document.createElement("div");
  hero.className = "hero";
  hero.textContent = spec.text;
  if (spec.h < spec.w * 0.5) {
    hero.style.fontSize = Math.round(Math.max(22, spec.h * 0.46)) + "px";
    hero.style.inset = "4% 6%";
  }
  root.appendChild(hero);
  const letters = design === "kinetic-wave" || design === "kinetic-burst";
  if (letters) {
    hero.textContent = "";
    hero.style.gap = "0.04em";
    for (const ch of spec.text) {
      const s = document.createElement("span");
      s.textContent = ch === " " ? "\\u00a0" : ch;
      s.style.display = "inline-block";
      hero.appendChild(s);
    }
  }
  window.seek = (t) => {
    const p = move(t);
    const b = back(Math.min(1, t / Math.min(1.1, spec.dur * 0.45)));
    if (design === "kinetic-typewriter") {
      const n = Math.round(spec.text.length * p);
      hero.textContent = spec.text.slice(0, n);
      hero.style.opacity = 1;
      hero.style.transform = "none";
      hero.style.filter = "none";
      return;
    }
    if (design === "kinetic-words") {
      hero.textContent = "";
      const bits = spec.text.split(/\\s+/);
      bits.forEach((word, i) => {
        const s = document.createElement("span");
        const local = ease((p * bits.length) - i);
        s.textContent = word + " ";
        s.style.display = "inline-block";
        s.style.opacity = local;
        s.style.transform = "translateY(" + ((1 - local) * 28) + "px)";
        hero.appendChild(s);
      });
      return;
    }
    if (design === "kinetic-wave") {
      [...hero.children].forEach((s, i) => {
        const y = Math.sin(p * 6 + i * 0.45) * 18 * (1 - p * 0.35);
        s.style.transform = "translateY(" + y + "px)";
        s.style.opacity = Math.min(1, p * 1.4);
      });
      return;
    }
    if (design === "kinetic-burst") {
      [...hero.children].forEach((s, i) => {
        const ang = i * 0.7;
        const dist = (1 - p) * 80;
        s.style.transform = "translate(" + (Math.cos(ang) * dist) + "px," + (Math.sin(ang) * dist) + "px)";
        s.style.opacity = p;
      });
      return;
    }
    if (design === "kinetic-glitch") {
      const shift = (1 - p) * 16;
      hero.style.opacity = 0.35 + p * 0.65;
      hero.style.transform = "translateX(" + ((iOffset(t) * shift)) + "px)";
      hero.style.textShadow = shift + "px 0 " + accent + ", " + (-shift) + "px 0 #67e8f9";
      hero.style.filter = "none";
      return;
    }
    if (design === "kinetic-blur") {
      hero.style.opacity = p;
      hero.style.filter = "blur(" + ((1 - p) * 18) + "px)";
      hero.style.transform = "scale(" + (1.08 - p * 0.08) + ")";
      return;
    }
    if (design === "kinetic-bounce") {
      const y = (1 - b) * -220;
      hero.style.opacity = Math.min(1, p * 1.4);
      hero.style.transform = "translateY(" + y + "px)";
      hero.style.filter = "none";
      return;
    }
    if (design === "kinetic-punch") {
      const s = 0.45 + b * 0.62;
      hero.style.opacity = Math.min(1, p * 1.5);
      hero.style.transform = "scale(" + s + ")";
      return;
    }
    if (design === "kinetic-editorial") {
      hero.style.fontSize = Math.round(spec.w * 0.07) + "px";
      hero.style.fontWeight = "560";
      hero.style.alignItems = "flex-end";
      hero.style.justifyContent = "flex-start";
      hero.style.textAlign = "left";
      hero.style.opacity = p;
      hero.style.transform = "translateY(" + ((1 - p) * 24) + "px)";
      return;
    }
    const y = (1 - b) * -160;
    hero.style.opacity = Math.min(1, p * 1.3);
    hero.style.transform = "translateY(" + y + "px)";
    hero.style.filter = "none";
    hero.style.textShadow = "none";
  };
}
function iOffset(t) { return Math.sin(t * 48) > 0 ? 1 : -1; }

function buildStat() {
  const value = nums()[0] ?? 100;
  const prefix = spec.text.includes("$") ? "$" : "";
  const suffix = spec.text.includes("%") ? "%" : "";
  const hero = document.createElement("div");
  hero.className = "hero";
  hero.style.fontVariantNumeric = "tabular-nums";
  hero.style.flexDirection = "column";
  hero.style.gap = "18px";
  const num = document.createElement("div");
  const finalShown = prefix + Math.round(value).toLocaleString("en-US") + suffix;
  const chars = Math.max(1, finalShown.length);
  const fitted = Math.max(28, Math.min(spec.w * 0.16, spec.w * 0.86 / (chars * 0.62)));
  const fittedH = spec.h < spec.w * 0.5 ? spec.h * 0.62 : fitted;
  num.style.fontSize = Math.round(Math.min(fitted, fittedH)) + "px";
  num.style.whiteSpace = "nowrap";
  num.style.lineHeight = "1";
  const label = document.createElement("div");
  label.style.fontSize = Math.round(spec.w * 0.035) + "px";
  label.style.letterSpacing = "0.08em";
  label.style.textTransform = "uppercase";
  label.style.color = muted;
  label.textContent = spec.text.replace(/[$%0-9.,]/g, "").trim();
  hero.appendChild(num);
  if (label.textContent) hero.appendChild(label);
  root.appendChild(hero);
  let ring;
  if (design === "stat-ring") {
    ring = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    ring.setAttribute("width", "280");
    ring.setAttribute("height", "280");
    ring.style.position = "absolute";
    ring.style.left = "50%";
    ring.style.top = "50%";
    ring.style.transform = "translate(-50%, -58%)";
    const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    c.setAttribute("cx", "140");
    c.setAttribute("cy", "140");
    c.setAttribute("r", "112");
    c.setAttribute("fill", "none");
    c.setAttribute("stroke", accent);
    c.setAttribute("stroke-width", "14");
    c.setAttribute("stroke-linecap", "round");
    const len = 2 * Math.PI * 112;
    c.dataset.len = String(len);
    ring.appendChild(c);
    root.appendChild(ring);
  }
  const bars = [];
  if (design === "stat-bars") {
    const row = document.createElement("div");
    row.style.position = "absolute";
    row.style.left = "12%";
    row.style.right = "12%";
    row.style.bottom = "16%";
    row.style.display = "flex";
    row.style.gap = "10px";
    row.style.alignItems = "flex-end";
    row.style.height = "28%";
    const series = (nums().length > 1 ? nums() : [30, 55, 80, 45]).slice(0, 6);
    const max = Math.max(...series, 1);
    series.forEach(() => {
      const b = document.createElement("div");
      b.style.flex = "1";
      b.style.background = accent;
      b.style.transformOrigin = "bottom";
      row.appendChild(b);
      bars.push({ el: b, h: 0 });
    });
    series.forEach((v, i) => { bars[i].h = v / max; });
    root.appendChild(row);
  }
  window.seek = (t) => {
    const p = move(t);
    const shown = Math.round(value * p);
    num.textContent = prefix + shown.toLocaleString("en-US") + suffix;
    if (ring) {
      const c = ring.querySelector("circle");
      const len = Number(c.dataset.len);
      c.style.strokeDasharray = len;
      c.style.strokeDashoffset = String(len * (1 - p));
      c.style.transform = "rotate(-90deg)";
      c.style.transformOrigin = "140px 140px";
    }
    bars.forEach((b, i) => {
      const local = ease(p * bars.length - i * 0.35);
      b.el.style.height = (local * b.h * 100) + "%";
    });
  };
}

function buildChart() {
  const series = (nums().length >= 2 ? nums() : [28, 64, 41, 86, 53]).slice(0, 6);
  const names = labels();
  const max = Math.max(...series, 1);
  const wrap = document.createElement("div");
  wrap.className = "chart";
  wrap.style.inset = "12%";
  const title = document.createElement("div");
  title.textContent = spec.text;
  title.style.fontSize = Math.round(spec.w * 0.045) + "px";
  title.style.fontWeight = "700";
  title.style.marginBottom = "24px";
  wrap.appendChild(title);
  root.appendChild(wrap);
  if (design === "chart-line") {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 600 320");
    svg.style.width = "100%";
    svg.style.height = "70%";
    const pts = series.map((v, i) => {
      const x = 30 + (i / Math.max(1, series.length - 1)) * 540;
      const y = 280 - (v / max) * 230;
      return [x, y];
    });
    const d = pts.map((p, i) => (i ? "L" : "M") + p[0] + " " + p[1]).join(" ");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", accent);
    path.setAttribute("stroke-width", "8");
    path.setAttribute("stroke-linecap", "round");
    svg.appendChild(path);
    wrap.appendChild(svg);
    window.seek = (t) => {
      const p = move(t);
      const len = path.getTotalLength ? path.getTotalLength() : 800;
      path.style.strokeDasharray = len;
      path.style.strokeDashoffset = String(len * (1 - p));
    };
    return;
  }
  if (design === "chart-pie") {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 200 200");
    svg.style.width = "58%";
    svg.style.height = "58%";
    const total = series.reduce((a, b) => a + b, 0) || 1;
    let angle = -Math.PI / 2;
    const slices = series.map((v, i) => {
      const sweep = (v / total) * Math.PI * 2;
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("fill", i === 0 ? accent : shade(accent, 0.45 + i * 0.1));
      svg.appendChild(path);
      const slice = { path, a0: angle, sweep };
      angle += sweep;
      return slice;
    });
    wrap.appendChild(svg);
    window.seek = (t) => {
      const p = move(t);
      slices.forEach((s) => {
        const a1 = s.a0 + s.sweep * p;
        s.path.setAttribute("d", wedge(100, 100, 86, s.a0, a1));
      });
    };
    return;
  }
  const row = document.createElement("div");
  row.style.display = "flex";
  row.style.alignItems = "flex-end";
  row.style.gap = "14px";
  row.style.height = "68%";
  const bars = series.map((v, i) => {
    const col = document.createElement("div");
    col.style.flex = "1";
    col.style.display = "flex";
    col.style.flexDirection = "column";
    col.style.justifyContent = "flex-end";
    col.style.height = "100%";
    const b = document.createElement("div");
    b.style.background = accent;
    b.style.width = "100%";
    const cap = document.createElement("div");
    cap.style.fontSize = "13px";
    cap.style.color = muted;
    cap.style.marginTop = "8px";
    cap.textContent = names[i] || String(v);
    col.appendChild(b);
    col.appendChild(cap);
    row.appendChild(col);
    return { el: b, v };
  });
  wrap.appendChild(row);
  window.seek = (t) => {
    const p = move(t);
    const order = design === "chart-race"
      ? bars.map((b, i) => ({ i, s: b.v * (0.35 + p * (0.4 + (i % 3) * 0.25)) })).sort((a, c) => c.s - a.s)
      : null;
    bars.forEach((b, i) => {
      const local = design === "chart-race" ? p : ease(p * 1.3 - i * 0.12);
      const score = design === "chart-race" ? order.find((o) => o.i === i).s / max : (b.v / max) * local;
      b.el.style.height = Math.max(4, score * 100) + "%";
    });
  };
}
function shade(hex, mix) {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  const m = Math.round(255 * (1 - mix));
  const c = (v) => Math.round(v * mix + m * (1 - mix));
  return "#" + [c(r), c(g), c(b)].map((v) => v.toString(16).padStart(2, "0")).join("");
}
function wedge(cx, cy, r, a0, a1) {
  if (a1 - a0 < 0.001) return "";
  const large = a1 - a0 > Math.PI ? 1 : 0;
  const x0 = cx + r * Math.cos(a0), y0 = cy + r * Math.sin(a0);
  const x1 = cx + r * Math.cos(a1), y1 = cy + r * Math.sin(a1);
  return "M " + cx + " " + cy + " L " + x0 + " " + y0 + " A " + r + " " + r + " 0 " + large + " 1 " + x1 + " " + y1 + " Z";
}

function buildLower() {
  const el = document.createElement("div");
  el.textContent = spec.text;
  el.style.position = "absolute";
  el.style.fontWeight = "700";
  if (design === "lower-split") {
    el.style.left = "0";
    el.style.top = "0";
    el.style.bottom = "0";
    el.style.width = "46%";
    el.style.background = accent;
    el.style.color = bg;
    el.style.display = "flex";
    el.style.alignItems = "center";
    el.style.padding = "8%";
    el.style.fontSize = Math.round(spec.w * 0.05) + "px";
    el.style.boxSizing = "border-box";
  } else if (design === "lower-quote") {
    el.style.left = "10%";
    el.style.right = "10%";
    el.style.top = "28%";
    el.style.fontSize = Math.round(spec.w * 0.06) + "px";
    el.style.fontWeight = "560";
    el.style.lineHeight = "1.15";
  } else if (design === "lower-callout") {
    el.style.left = "10%";
    el.style.bottom = "16%";
    el.style.padding = "18px 26px";
    el.style.background = accent;
    el.style.color = bg;
    el.style.borderRadius = "18px";
    el.style.fontSize = Math.round(spec.w * 0.04) + "px";
  } else {
    el.style.left = "0";
    el.style.right = "0";
    el.style.bottom = "11%";
    el.style.padding = "22px 7%";
    el.style.background = accent;
    el.style.color = bg;
    el.style.fontSize = Math.round(spec.w * 0.045) + "px";
  }
  root.appendChild(el);
  window.seek = (t) => {
    const p = move(t);
    if (design === "lower-split") el.style.transform = "translateX(" + ((1 - p) * -100) + "%)";
    else if (design === "lower-quote") { el.style.opacity = p; el.style.transform = "translateY(" + ((1 - p) * 20) + "px)"; }
    else if (design === "lower-callout") { el.style.opacity = p; el.style.transform = "translateY(" + ((1 - p) * 28) + "px) scale(" + (0.92 + p * 0.08) + ")"; }
    else el.style.transform = "scaleX(" + p + ")";
    el.style.transformOrigin = "left center";
  };
}

function buildLogo() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 200 200");
  svg.style.position = "absolute";
  svg.style.width = "28%";
  svg.style.left = "36%";
  svg.style.top = "22%";
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M30 110 L80 160 L170 40");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke", accent);
  path.setAttribute("stroke-width", "16");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  svg.appendChild(path);
  const word = document.createElement("div");
  word.className = "hero";
  word.style.top = "58%";
  word.style.fontSize = Math.round(spec.w * 0.07) + "px";
  word.textContent = spec.text;
  root.appendChild(svg);
  root.appendChild(word);
  const blocks = [];
  if (design === "logo-lockup") {
    path.style.display = "none";
    for (let i = 0; i < 3; i++) {
      const b = document.createElement("div");
      b.style.position = "absolute";
      b.style.top = "24%";
      b.style.left = (28 + i * 16) + "%";
      b.style.width = "12%";
      b.style.height = "18%";
      b.style.background = i === 1 ? accent : fg;
      root.appendChild(b);
      blocks.push(b);
    }
  }
  window.seek = (t) => {
    const p = move(t);
    const len = 360;
    path.style.strokeDasharray = len;
    path.style.strokeDashoffset = String(len * (1 - p));
    word.style.opacity = Math.max(0, (p - 0.45) / 0.55);
    blocks.forEach((b, i) => {
      const local = ease(p * 1.4 - i * 0.18);
      b.style.transform = "translateY(" + ((1 - local) * -80) + "px)";
      b.style.opacity = local;
    });
  };
}

function buildMap() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 640 400");
  svg.style.position = "absolute";
  svg.style.inset = "16% 8% 18% 8%";
  svg.style.width = "84%";
  svg.style.height = "58%";
  const blobs = [[80, 90, 120, 70], [220, 70, 150, 90], [390, 120, 160, 80], [180, 190, 200, 90]];
  blobs.forEach(([x, y, w, h], i) => {
    const e = document.createElementNS("http://www.w3.org/2000/svg", "ellipse");
    e.setAttribute("cx", x + w / 2);
    e.setAttribute("cy", y + h / 2);
    e.setAttribute("rx", w / 2);
    e.setAttribute("ry", h / 2);
    e.setAttribute("fill", i === 2 ? accent : muted);
    e.setAttribute("opacity", "0.35");
    e.dataset.i = String(i);
    svg.appendChild(e);
  });
  const route = document.createElementNS("http://www.w3.org/2000/svg", "path");
  route.setAttribute("d", "M120 140 C 220 80, 300 220, 500 160");
  route.setAttribute("fill", "none");
  route.setAttribute("stroke", fg);
  route.setAttribute("stroke-width", "6");
  svg.appendChild(route);
  root.appendChild(svg);
  const caption = document.createElement("div");
  caption.className = "hero";
  caption.style.top = "72%";
  caption.style.fontSize = Math.round(spec.w * 0.05) + "px";
  caption.textContent = spec.text;
  root.appendChild(caption);
  window.seek = (t) => {
    const p = move(t);
    [...svg.querySelectorAll("ellipse")].forEach((e) => {
      const on = e.dataset.i === "2";
      e.setAttribute("opacity", on ? String(0.25 + p * 0.75) : "0.28");
    });
    if (design === "map-route") {
      const len = 520;
      route.style.strokeDasharray = len;
      route.style.strokeDashoffset = String(len * (1 - p));
      route.style.opacity = 1;
    } else {
      route.style.opacity = 0;
    }
    caption.style.opacity = Math.max(0, (p - 0.4) / 0.6);
  };
}

window.seek(0);
</script>
</body>
</html>`;
}

function pickBin(envName, candidates, fallback) {
  const fromEnv = process.env[envName];
  if (fromEnv) return fromEnv;
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  return fallback;
}

function chromeBin() {
  return pickBin("CHROME_PATH", ["/usr/bin/google-chrome", "/usr/bin/chromium"], "google-chrome");
}

function ffmpegBin() {
  return pickBin("FFMPEG_PATH", ["/usr/bin/ffmpeg"], "ffmpeg");
}

// A finished graphic must not fail because Chrome's helpers were still writing
// Default/ when the profile directory was removed.
export function chromeLaunchFailed(err) {
  const code = err && err.code ? String(err.code) : "";
  const text = `${code} ${err && err.message ? err.message : err}`;
  return /ENOTEMPTY|EBUSY|EPERM|Directory not empty|chrome exited before debugging|did not open a debugging port|chrome socket failed|ECONNREFUSED|ECONNRESET|fetch failed/.test(
    text,
  );
}

function descendantPids(pid) {
  let kids = [];
  try {
    kids = readFileSync(`/proc/${pid}/task/${pid}/children`, "utf8").trim().split(/\s+/).filter(Boolean);
  } catch {
    return [];
  }
  const out = [];
  for (const kid of kids) {
    const n = Number(kid);
    if (!n) continue;
    out.push(n, ...descendantPids(n));
  }
  return out;
}

function stopChrome(child) {
  if (!child || !child.pid) return;
  const tree = descendantPids(child.pid);
  for (const pid of tree.reverse()) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      /* already gone */
    }
  }
  try {
    child.kill("SIGKILL");
  } catch {
    /* already gone */
  }
}

function chromeGone(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, 500);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function removeTree(dir) {
  try {
    await rm(dir, { recursive: true, force: true, maxRetries: 8, retryDelay: 40 });
  } catch {
    // Leftover /tmp files are not the graphic.
  }
}

function listen(child) {
  return new Promise((resolve, reject) => {
    let buf = "";
    let settled = false;
    let timer;
    const finish = (fn) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.stderr.off("data", onData);
      child.off("exit", onExit);
      fn();
    };
    const onData = (chunk) => {
      buf += chunk.toString();
      if (buf.length > 8000) buf = buf.slice(-4000);
      const found = buf.match(/DevTools listening on ws:\/\/(?:127\.0\.0\.1|\[::1\]):(\d+)/);
      if (found) finish(() => resolve(found[1]));
    };
    const onExit = (code) => {
      const tail = buf.trim().split("\n").slice(-4).join(" ").slice(0, 500);
      finish(() => reject(new Error(`chrome exited before debugging: ${code}${tail ? ` ${tail}` : ""}`)));
    };
    timer = setTimeout(() => finish(() => reject(new Error("chrome did not open a debugging port"))), 15000);
    child.stderr.on("data", onData);
    child.on("exit", onExit);
  });
}

function cdp(ws) {
  let n = 0;
  const pending = new Map();
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
      else resolve(msg.result || {});
    }
  });
  return (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const id = ++n;
      pending.set(id, { resolve, reject });
      const payload = { id, method, params };
      if (sessionId) payload.sessionId = sessionId;
      ws.send(JSON.stringify(payload));
    });
}

async function shoot(html, dir, frames, w, h, alpha) {
  let last;
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      await shootOnce(html, dir, frames, w, h, alpha);
      return;
    } catch (err) {
      last = err;
      if (attempt === 1 || !chromeLaunchFailed(err)) throw err;
    }
  }
  throw last;
}

async function shootOnce(html, dir, frames, w, h, alpha) {
  const server = createServer((req, res) => {
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    res.end(html);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  const url = `http://127.0.0.1:${port}/`;
  const userDir = await mkdtemp(path.join(tmpdir(), "oc-chrome-"));
  let chrome;
  let ws;
  try {
    chrome = spawn(
      chromeBin(),
      [
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--hide-scrollbars",
        "--disable-breakpad",
        "--disable-crash-reporter",
        "--remote-debugging-port=0",
        `--user-data-dir=${userDir}`,
        "--no-first-run",
        "about:blank",
      ],
      { stdio: ["ignore", "ignore", "pipe"] },
    );
    const dbg = await listen(chrome);
    const version = await fetch(`http://127.0.0.1:${dbg}/json/version`).then((r) => r.json());
    ws = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve);
      ws.addEventListener("error", () => reject(new Error("chrome socket failed")));
    });
    const send = cdp(ws);
    const { targetId } = await send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
    await send("Page.enable", {}, sessionId);
    await send("Runtime.enable", {}, sessionId);
    await send(
      "Emulation.setDeviceMetricsOverride",
      { width: w, height: h, deviceScaleFactor: 1, mobile: false },
      sessionId,
    );
    if (alpha) {
      await send(
        "Emulation.setDefaultBackgroundColorOverride",
        { color: { r: 0, g: 0, b: 0, a: 0 } },
        sessionId,
      );
    }
    await send("Page.navigate", { url }, sessionId);
    await new Promise((r) => setTimeout(r, 250));
    const ext = alpha ? "png" : "jpg";
    for (let i = 0; i < frames; i++) {
      const t = frames === 1 ? 0 : (i / (frames - 1)) * Number(arg("--dur", "4"));
      await send(
        "Runtime.evaluate",
        { expression: `window.seek(${t})`, returnByValue: true },
        sessionId,
      );
      const shot = await send(
        "Page.captureScreenshot",
        alpha
          ? { format: "png", omitBackground: true, clip: { x: 0, y: 0, width: w, height: h, scale: 1 } }
          : { format: "jpeg", quality: 82 },
        sessionId,
      );
      await writeFile(path.join(dir, `f${String(i).padStart(4, "0")}.${ext}`), Buffer.from(shot.data, "base64"));
    }
    ws.close();
  } finally {
    if (ws) {
      try {
        ws.close();
      } catch {
        /* already closed */
      }
    }
    stopChrome(chrome);
    server.close();
    await chromeGone(chrome);
    await removeTree(userDir);
  }
}

function encode(dir, out, frames, alpha) {
  const input = path.join(dir, alpha ? "f%04d.png" : "f%04d.jpg");
  const codec = alpha
    ? ["-vf", "format=yuva420p", "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-auto-alt-ref", "0", "-lag-in-frames", "0", "-b:v", "0", "-crf", "28", "-an", "-metadata:s:v:0", "alpha_mode=1"]
    : ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart"];
  return new Promise((resolve, reject) => {
    const ff = spawn(ffmpegBin(), [
      "-y",
      "-hide_banner",
      "-loglevel",
      "error",
      "-framerate",
      String(FPS),
      "-i",
      input,
      "-frames:v",
      String(frames),
      ...codec,
      out,
    ]);
    let err = "";
    ff.stderr.on("data", (c) => {
      err += c.toString();
    });
    ff.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(err || "ffmpeg failed"));
    });
  });
}

async function main() {
  const out = arg("--out");
  const design = arg("--design");
  const dur = Math.max(0.4, Number(arg("--dur", "4")) || 4);
  const w = Number(arg("--w", "1280"));
  const h = Number(arg("--h", "720"));
  if (!out || !design) {
    console.error("render.mjs needs --out and --design");
    process.exit(1);
  }
  const alpha = h < w * 0.5;
  const spec = {
    design,
    text: arg("--text", " "),
    prompt: arg("--prompt", ""),
    style: arg("--style", "bold"),
    dur,
    w,
    h,
  };
  const frames = Math.max(4, Math.round(dur * FPS));
  const dir = await mkdtemp(path.join(tmpdir(), "oc-motion-"));
  try {
    await shoot(page(spec), dir, frames, w, h, alpha);
    await encode(dir, out, frames, alpha);
  } catch (err) {
    console.error(String(err && err.stack ? err.stack : err));
    process.exit(1);
  } finally {
    await removeTree(dir);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
