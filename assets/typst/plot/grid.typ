#chart((p, pad, top-y) => {
  import cetz.draw: *
  let end-alignment = right
  let rows = p.rows
  let cols = p.series.len()
  let row-label-width = p.x_ticks.map(t => measure(text(t.label)).width / u).fold(0, calc.max)
  let left = pad + calc.min(110, row-label-width) + 12
  let legend-width = p.y_ticks.map(t => measure(text(t.label)).width / u).fold(24, calc.max)
  let right = w - pad - legend-width - 32
  let pw = right - left
  let cw = pw / cols
  let column-stride = calc.max(1, calc.ceil(base * 3 / cw))
  let column-label(series) = block(width: calc.max(1, calc.min(100, cw * column-stride - 4)) * u, {
    set par(leading: base * 0.3 * u)
    align(center, text(fill: muted, series.label.clusters().join("\u{200b}")))
  })
  let label-height = p.series.enumerate().filter(((col, series)) => calc.rem(col, column-stride) == 0)
    .map(((col, series)) => measure(column-label(series)).height / u).fold(0, calc.max)
  let bottom = pad + label-height + 12
  let ph = top-y - bottom
  assert(pw >= 40 and ph >= 40, message: "The grid chart labels leave too little space; shorten the labels")
  let ch = ph / rows
  let gap = calc.min(1, cw * 0.08, ch * 0.08)
  let bins = range(64).map(_ => ())
  let missing = ()
  let cell-path(col, row) = {
    let x = left + col * cw + gap / 2
    let y = bottom + row * ch + gap / 2
    ((x, y, 0), true, (("l", (x + cw - gap, y, 0)), ("l", (x + cw - gap, y + ch - gap, 0)), ("l", (x, y + ch - gap, 0))))
  }
  for (col, series) in p.series.enumerate() {
    for (row, point) in series.points.enumerate() {
      let path = cell-path(col, row)
      if point == none { missing.push(path) }
      else {
        let bin = int(calc.round(calc.clamp(point.at(1), 0, 1) * 63))
        bins.at(bin).push(path)
      }
    }
  }
  let shade(value) = {
    let accent = color(cfg.style.colors.first())
    let low = mix-colors((bg, 92%), (accent, 8%))
    if p.zero > 0 and p.zero < 1 {
      if value < p.zero { mix-colors((low, value / p.zero), (color(cfg.style.colors.at(4)), 1 - value / p.zero)) }
      else { mix-colors((low, 1 - (value - p.zero) / (1 - p.zero)), (accent, (value - p.zero) / (1 - p.zero))) }
    } else { mix-colors((low, 1 - value), (accent, value)) }
  }
  draw-paths(missing, fill: mix-colors((bg, 75%), (grid-color, 25%)))
  for (index, paths) in bins.enumerate() { draw-paths(paths, fill: shade(index / 63)) }
  for (col, series) in p.series.enumerate() {
    if calc.rem(col, column-stride) == 0 {
      content((left + (col + 0.5) * cw, bottom - 12), column-label(series), anchor: "north", padding: 0)
    }
  }
  let row-stride = calc.max(1, calc.ceil((base + 5) / ch))
  for (row, tick) in p.x_ticks.enumerate() {
    if calc.rem(row, row-stride) == 0 {
      let label = block(width: calc.min(110, row-label-width) * u, height: base * 1.4 * u, clip: true,
        align(end-alignment, text(fill: muted, tick.label)))
      content((left - 12, bottom + (row + 0.5) * ch), label, anchor: "east", padding: 0)
    }
  }
  for index in range(64) {
    rect((right + 16, bottom + index / 64 * ph), (right + 24, bottom + calc.min(ph, (index + 1) / 64 * ph + 0.5)), stroke: none, fill: shade(index / 63))
  }
  for tick in p.y_ticks {
    content((right + 31, bottom + tick.position * ph), text(size: base * 0.85 * u, fill: muted, tick.label), anchor: "west", padding: 0)
  }
  if missing != () {
    rect((right + 16, pad), (right + 24, pad + 7), stroke: none, fill: mix-colors((bg, 75%), (grid-color, 25%)))
    content((right + 31, pad + 3.5), text(size: base * 0.75 * u, fill: muted, "NULL"), anchor: "west", padding: 0)
  }
})
