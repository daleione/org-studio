#import "@preview/cetz:0.5.2"
#let cfg = json(bytes(sys.inputs.at("chart-json")))
#let u = 0.75pt
#let w = cfg.width
#let h = cfg.height
#let base = cfg.style.font_size_100 / 100
#let color-mix = color.mix
#let mix-colors(..colors) = color-mix(..colors, space: rgb)
#let color(value) = rgb(
  calc.rem(calc.floor(value / 65536), 256),
  calc.rem(calc.floor(value / 256), 256),
  calc.rem(value, 256),
)
#let bg = color(cfg.style.background)
#let fg = color(cfg.style.foreground)
#let muted = color(cfg.style.muted)
#let grid-color = color(cfg.style.grid)
#set page(width: w * u, height: h * u, margin: 0pt, fill: none)
#set text(font: cfg.style.fonts, size: base * u, fill: fg)
#set par(leading: 0pt, spacing: 0pt)
#let draw-paths(paths, stroke: none, fill: none) = if paths != () {
  ((ctx) => (ctx: ctx, drawables: cetz.drawable.path(paths, stroke: stroke, fill: fill)),)
}
#let marker-path(at, index) = {
  let r = 2.7
  let (x, y) = at
  if calc.rem(index, 4) == 0 {
    let k = 0.551784 * r
    ((x, y + r, 0), true, (
      ("c", (x - k, y + r, 0), (x - r, y + k, 0), (x - r, y, 0)),
      ("c", (x - r, y - k, 0), (x - k, y - r, 0), (x, y - r, 0)),
      ("c", (x + k, y - r, 0), (x + r, y - k, 0), (x + r, y, 0)),
      ("c", (x + r, y + k, 0), (x + k, y + r, 0), (x, y + r, 0)),
    ))
  } else {
    let vertices = if calc.rem(index, 4) == 1 {
      ((x, y + r, 0), (x + r, y, 0), (x, y - r, 0), (x - r, y, 0))
    } else if calc.rem(index, 4) == 2 {
      ((x - r, y - r, 0), (x + r, y - r, 0), (x + r, y + r, 0), (x - r, y + r, 0))
    } else { ((x, y + r, 0), (x + r, y - r, 0), (x - r, y - r, 0)) }
    (vertices.first(), true, vertices.slice(1).map(pt => ("l", pt)))
  }
}
#let marker(at, index, paint, filled: false) = draw-paths(
  (marker-path(at, index),), stroke: (paint: paint, thickness: 1.6 * u), fill: if filled { paint } else { bg },
)
#let dashes = ("solid", "dashed", "dotted", "dash-dotted", "densely-dashed", "loosely-dotted", "densely-dash-dotted", "loosely-dashed")
#let chart(body) = context place(top + left, cetz.canvas(length: u, {
  import cetz.draw: *
  let p = cfg.plan
  let pad = calc.min(28, w * 0.065)
  rect((0, 0), (w, h), fill: bg, stroke: none)
  let top-y = h - pad
  if p.title != none and p.title != "" {
    let title = block(width: (w - 2 * pad) * u, text(size: base * 1.75 * u, weight: "semibold", p.title))
    content((pad, top-y), title, anchor: "north-west", padding: 0)
    top-y -= measure(title).height / u + 16
  }
  if p.series.len() > 1 and p.layout != "grid" {
    let legend-x = pad
    let legend-y = top-y - base / 2
    for (i, series) in p.series.enumerate() {
      let width = measure(text(size: base * u, series.label)).width / u + 32
      assert(width <= w - 2 * pad, message: "A series label is too long for the chart; shorten the label")
      if legend-x + width > w - pad {
        legend-x = pad
        legend-y -= base + 10
      }
      if p.kind in ("lines", "steps", "f_steps", "hi_steps", "filled_curves", "impulses") or p.layout == "radar" {
        line((legend-x, legend-y), (legend-x + 10, legend-y), stroke: (paint: color(cfg.style.colors.at(i)), thickness: 2.7 * u, dash: dashes.at(i), cap: "round"))
      } else {
        marker((legend-x + 4, legend-y), i, color(cfg.style.colors.at(i)), filled: p.kind == "histograms" or (p.kind == "points" and i < 4) or (p.kind == "lines_points" and i >= 4))
      }
      content((legend-x + 15, legend-y), text(size: base * u, fill: muted, series.label), anchor: "west", padding: 0)
      legend-x += width
    }
    top-y = legend-y - base / 2 - 20
  }
  body(p, pad, top-y)
}))
