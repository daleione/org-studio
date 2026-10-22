#chart((p, pad, top-y) => {
  import cetz.draw: *
  let bars = p.kind == "histograms"
  let start-alignment = left
  let end-alignment = right
  let y-width = p.y_ticks.map(t => measure(text(t.label)).width / u).fold(0, calc.max)
  let left = pad + 5
  let right = w - pad - y-width - 12
  let pw = right - left
  assert(pw > 40, message: "The chart is too narrow for its axis labels")
  let slot = pw / calc.max(1, p.rows)
  let max-labels = calc.max(2, calc.floor(pw / (base * 4)))
  let stride = calc.max(1, calc.ceil(p.x_ticks.len() / max-labels))
  let ticks = p.x_ticks.enumerate()
    .filter(((i, t)) => calc.rem(i, stride) == 0 or (not bars and i == p.x_ticks.len() - 1))
    .map(((i, t)) => t)
    .filter(t => not bars or pw * (1 - t.position) + slot / 2 >= base * 2)
  let label-inset = calc.min(base / 2, slot * stride * 0.12)
  let label-width = if bars {
    calc.min(120, slot * stride - 2 * label-inset)
  } else { calc.min(120, pw / calc.max(1, ticks.len()) * 0.95) }
  let label-span(tick) = {
    let available = if bars { pw * (1 - tick.position) + slot / 2 - 2 * label-inset } else { label-width }
    calc.max(1, calc.min(label-width, available, measure(text(size: base * u, tick.label)).width / u))
  }
  let label-body(tick, alignment: center) = block(width: label-span(tick) * u, {
    set par(leading: base * 0.3 * u)
    align(alignment, text(size: base * u, fill: muted, tick.label.clusters().join("\u{200b}")))
  })
  let label-height = ticks.map(t => measure(label-body(t)).height / u).fold(0, calc.max)
  let bottom = pad + label-height + 12
  let ph = top-y - bottom
  assert(ph >= 50, message: "The chart labels leave too little space to draw; shorten the labels or use fewer series")
  if bars {
    let divisions = ()
    for i in range(0, p.rows + 1, step: int(stride)) {
      let x = left + i * slot
      divisions.push(((x, top-y, 0), false, (("l", (x, pad - 3, 0)),)))
    }
    if calc.rem(p.rows, stride) != 0 {
      divisions.push(((right, top-y, 0), false, (("l", (right, pad - 3, 0)),)))
    }
    draw-paths(divisions, stroke: (paint: grid-color, thickness: 0.75 * u, dash: (3 * u, 3 * u)))
  }
  for tick in p.y_ticks {
    let y = bottom + tick.position * ph
    let thickness = if bars and calc.abs(tick.position - p.zero) < 0.000001 { 1 } else { 0.75 }
    line((left, y), (right, y), stroke: (paint: grid-color, thickness: thickness * u))
    content((right + 10, y), text(size: base * u, fill: muted, tick.label), anchor: "west", padding: 0)
  }
  let last-label-right = -1000
  for tick in ticks {
    let x = left + tick.position * pw
    let span = label-span(tick)
    let near-left = x - span / 2 < pad
    let near-right = x + span / 2 > w - pad
    let label-left = if bars { x - slot / 2 + label-inset } else if near-left { x } else if near-right { x - span } else { x - span / 2 }
    if label-left >= last-label-right + 3 {
      if bars {
        content((label-left, bottom - 12), label-body(tick, alignment: start-alignment), anchor: "north-west", padding: 0)
      } else {
        content((x, bottom - 12), label-body(tick, alignment: if near-left { start-alignment } else if near-right { end-alignment } else { center }), anchor: if near-left { "north-west" } else if near-right { "north-east" } else { "north" }, padding: 0)
      }
      last-label-right = label-left + span
    }
  }
  let marks = block(width: pw * u, height: ph * u, clip: p.clip, cetz.canvas(length: u, {
    import cetz.draw: *
    rect((0, 0), (pw, ph), fill: none, stroke: none)
    floating({
      let left = 0
      let bottom = 0
      for (i, series) in p.series.enumerate() {
        let paint = color(cfg.style.colors.at(i))
        let stroke = (paint: paint, thickness: 2.7 * u, cap: "round", join: "round", dash: dashes.at(i))
        if bars {
          let count = p.series.len()
          let group = calc.min(slot * 0.64, count * 48 + (count - 1) * 4)
          let gap = calc.min(4, group / count * 0.12)
          let bw = (group - gap * (count - 1)) / count
          let zero = bottom + p.zero * ph
          let bars = ()
          for point in series.points {
            if point != none {
              let x = left + point.at(0) * pw - group / 2 + i * (bw + gap)
              let y = bottom + point.at(1) * ph
              if calc.abs(y - zero) > 0.001 {
                let r = calc.min(2, bw / 3, calc.abs(y - zero) / 3)
                let sign = if y > zero { 1 } else { -1 }
                let k = r * (1 - 0.551784)
                bars.push(((x, zero, 0), true, (
                  ("l", (x + bw, zero, 0)), ("l", (x + bw, y - sign * r, 0)),
                  ("c", (x + bw, y - sign * k, 0), (x + bw - k, y, 0), (x + bw - r, y, 0)),
                  ("l", (x + r, y, 0)),
                  ("c", (x + k, y, 0), (x, y - sign * k, 0), (x, y - sign * r, 0)),
                )))
              }
            }
          }
          draw-paths(bars, fill: paint)
        } else if p.kind == "boxes" {
          let paths = ()
          for (j, point) in series.points.enumerate() {
            if point != none and p.box_bounds.at(j) != none {
              let (a, b) = p.box_bounds.at(j)
              let x1 = left + a * pw
              let x2 = left + b * pw
              let y = bottom + point.at(1) * ph
              let zero = bottom + p.zero * ph
              paths.push(((x1, zero, 0), true, (("l", (x2, zero, 0)), ("l", (x2, y, 0)), ("l", (x1, y, 0)))))
            }
          }
          draw-paths(paths, stroke: (paint: paint, thickness: 0.8 * u), fill: paint.transparentize(if p.series.len() > 1 { 45% } else { 10% }))
        } else if p.kind == "impulses" {
          let paths = series.points.filter(point => point != none).map(point => {
            let x = left + point.at(0) * pw
            ((x, bottom + p.zero * ph, 0), false, (("l", (x, bottom + point.at(1) * ph, 0)),))
          })
          draw-paths(paths, stroke: stroke)
        } else {
          if p.kind != "points" {
            let segment = ()
            let paths = ()
            for point in series.points + (none,) {
              if point == none {
                if segment.len() > 1 {
                  let vertices = (segment.first(),)
                  if p.kind == "hi_steps" {
                    vertices = ((segment.first().at(0) - (segment.at(1).at(0) - segment.first().at(0)) / 2, segment.first().at(1), 0), segment.first())
                  }
                  for j in range(1, segment.len()) {
                    let a = segment.at(j - 1)
                    let b = segment.at(j)
                    if p.kind == "steps" { vertices.push((b.at(0), a.at(1), 0)) }
                    else if p.kind == "f_steps" { vertices.push((a.at(0), b.at(1), 0)) }
                    else if p.kind == "hi_steps" {
                      let mid = (a.at(0) + b.at(0)) / 2
                      vertices.push((mid, a.at(1), 0))
                      vertices.push((mid, b.at(1), 0))
                    }
                    vertices.push(b)
                  }
                  if p.kind == "hi_steps" {
                    let last = segment.last()
                    vertices.push((last.at(0) + (last.at(0) - segment.at(-2).at(0)) / 2, last.at(1), 0))
                  }
                  if p.kind == "filled_curves" and p.fill_to_zero {
                    vertices.push((segment.last().at(0), bottom + p.zero * ph, 0))
                    vertices.push((segment.first().at(0), bottom + p.zero * ph, 0))
                  }
                  paths.push((vertices.first(), p.kind == "filled_curves", vertices.slice(1).map(pt => ("l", pt))))
                }
                segment = ()
              } else { segment.push((left + point.at(0) * pw, bottom + point.at(1) * ph, 0)) }
            }
            draw-paths(paths, stroke: stroke, fill: if p.kind == "filled_curves" { paint.transparentize(82%) } else { none })
          }
          if p.kind in ("points", "lines_points") and series.points.any(point => point != none) {
            let filled = (p.kind == "points" and i < 4) or (p.kind == "lines_points" and i >= 4)
            let paths = series.points.filter(point => point != none).map(point =>
              marker-path((left + point.at(0) * pw, bottom + point.at(1) * ph), i))
            draw-paths(paths, stroke: (paint: paint, thickness: 1.6 * u), fill: if filled { paint } else { bg })
          }
        }
      }
    })
  }))
  content((left, bottom), marks, anchor: "south-west", padding: 0)
})
