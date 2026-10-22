#chart((p, pad, top-y) => {
  import cetz.draw: *
  let same-scales = p.radar_ticks.all(ticks => ticks == p.radar_ticks.first())
  let gap = 10
  let axes = p.x_ticks.enumerate().map(((index, tick)) => {
    let angle = 90deg - index * 360deg / p.rows
    let x = calc.cos(angle)
    let y = calc.sin(angle)
    let width = calc.max(1, calc.min(100, w * 0.26, measure(text(tick.label)).width / u))
    let label = block(width: width * u, {
      set par(leading: base * 0.3 * u)
      align(center, text(fill: muted, tick.label.clusters().join("\u{200b}")))
    })
    let size = measure(label)
    let anchor = if calc.abs(x) < 0.01 { if y > 0 { "south" } else { "north" } }
      else if x > 0 { "west" } else { "east" }
    (direction: (x, y), label: label, size: (size.width / u, size.height / u), anchor: anchor)
  })
  let bounds(radius) = {
    let (min-x, min-y, max-x, max-y) = (0, 0, 0, 0)
    for axis in axes {
      let (x, y) = axis.direction
      let (lw, lh) = axis.size
      let label-x = x * (radius + gap) + if axis.anchor == "west" { 0 } else if axis.anchor == "east" { -lw } else { -lw / 2 }
      let label-y = y * (radius + gap) + if axis.anchor == "south" { 0 } else if axis.anchor == "north" { -lh } else { -lh / 2 }
      min-x = calc.min(min-x, x * radius - 4, label-x)
      max-x = calc.max(max-x, x * radius + 4, label-x + lw)
      min-y = calc.min(min-y, y * radius - 4, label-y)
      max-y = calc.max(max-y, y * radius + 4, label-y + lh)
    }
    (min-x, min-y, max-x, max-y)
  }
  let width = w - 2 * pad
  let height = top-y - pad
  let radius = 0
  let limit = calc.max(w, h)
  for _ in range(16) {
    let candidate = (radius + limit) / 2
    let (x1, y1, x2, y2) = bounds(candidate)
    if x2 - x1 <= width and y2 - y1 <= height { radius = candidate }
    else { limit = candidate }
  }
  assert(radius >= 24, message: "The radar chart labels leave too little space; shorten the labels or use fewer series")
  let (x1, y1, x2, y2) = bounds(radius)
  let center-x = pad + (width - (x2 - x1)) / 2 - x1
  let center-y = pad + (height - (y2 - y1)) / 2 - y1
  let at(index, value) = {
    let (x, y) = axes.at(index).direction
    (center-x + x * radius * value, center-y + y * radius * value, 0)
  }
  let ring-count = if p.radar_ticks.first() == () { 2 } else { p.radar_ticks.first().len() - 1 }
  for level in range(1, ring-count + 1) {
    let vertices = range(p.rows).map(index => at(index, level / ring-count))
    draw-paths(((vertices.first(), true, vertices.slice(1).map(pt => ("l", pt))),),
      stroke: (paint: grid-color, thickness: 0.75 * u))
  }
  for (index, ticks) in p.radar_ticks.enumerate() {
    line((center-x, center-y), at(index, 1), stroke: (paint: grid-color, thickness: 0.75 * u))
    let axis = axes.at(index)
    let (x, y) = axis.direction
    let label-at = (center-x + x * (radius + gap), center-y + y * (radius + gap))
    content(label-at, axis.label, anchor: axis.anchor, padding: 0)
    if not same-scales or index == 0 {
      for tick in ticks.filter(t => t.position > 0 or index == 0) {
        let point = at(index, tick.position)
        content(if same-scales { (point.at(0) - 6, point.at(1)) } else { (point.at(0) - y * 4, point.at(1) + x * 4) },
          box(fill: bg.transparentize(12%), inset: 1 * u, text(size: base * 0.72 * u, fill: muted, tick.label)),
          anchor: if same-scales { "east" } else { "south-west" }, padding: 0)
      }
    }
  }
  for (i, series) in p.series.enumerate() {
    let paint = color(cfg.style.colors.at(i))
    let complete = series.points.all(point => point != none)
    let segment = ()
    let paths = ()
    let points = series.points + if complete { () } else { (series.points.first(), none) }
    for (index, point) in points.enumerate() {
      if point == none {
        if segment.len() > 1 { paths.push((segment.first(), false, segment.slice(1).map(pt => ("l", pt)))) }
        segment = ()
      } else { segment.push(at(calc.rem(index, p.rows), point.at(1))) }
    }
    if complete {
      paths.push((segment.first(), true, segment.slice(1).map(pt => ("l", pt))))
    }
    draw-paths(paths, stroke: (paint: paint, thickness: 2 * u, dash: dashes.at(i), join: "round"),
      fill: if complete { paint.transparentize(88%) } else { none })
    let dots = series.points.enumerate().filter(((index, point)) => point != none)
      .map(((index, point)) => marker-path(at(index, point.at(1)).slice(0, 2), i))
    draw-paths(dots, stroke: (paint: paint, thickness: 1.5 * u), fill: bg)
  }
})
