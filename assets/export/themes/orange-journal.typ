// ═══════════════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════════════

#let paged = sys.inputs.at("paged", default: "false") == "true"
#let paper = sys.inputs.at("paper", default: "")

#let p-fscale  = float(sys.inputs.at("font-scale", default: "1.0"))
#let p-layout-scale = float(sys.inputs.at("layout-scale", default: "1.0"))
#let p-orient  = sys.inputs.at("orientation", default: "")      // portrait|landscape
#let p-indent  = sys.inputs.at("indent", default: "")
#let p-justify = sys.inputs.at("justify", default: "")
#let p-pagenum = sys.inputs.at("page-number", default: "")
#let p-footer  = sys.inputs.at("footer-text", default: "")

#let m-factor = float(sys.inputs.at("margin-scale", default: "1.0"))
#let l-factor = float(sys.inputs.at("line-height-scale", default: "1.0"))

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#3a1f18"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#a68676"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#fef7f3"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#fff2e9"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#f0d8c8"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#ffa94d"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#ff6b8b"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "28"))
#let size-h2      = float(sys.inputs.at("theme-size-h2",      default: "23"))
#let size-h3      = float(sys.inputs.at("theme-size-h3",      default: "21"))
#let size-h4      = float(sys.inputs.at("theme-size-h4",      default: "17"))
#let size-h5      = float(sys.inputs.at("theme-size-h5",      default: "15"))
#let size-h6      = float(sys.inputs.at("theme-size-h6",      default: "14"))
#let size-body    = float(sys.inputs.at("theme-size-body",    default: "15"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let size-code    = float(sys.inputs.at("theme-size-code",    default: "14"))
#let long-page-width      = 540pt
#let margin-x        = 39pt
#let margin-top      = 60pt
#let margin-bot      = 48pt
#let long-page-min-height = 720pt
#let content-width   = long-page-width - margin-x * 2
#let logo-url = ""

#let font-text  = (sys.inputs.at("theme-font-body", default: "Helvetica Neue"), "PingFang SC")
#let font-bold  = (sys.inputs.at("theme-font-heading", default: "Helvetica Neue"), "PingFang SC", "PingFang SC")
#let font-mono  = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "PingFang SC", "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg       = theme-bg
#let color-text     = theme-ink
#let color-soft     = rgb("#6f4a3e")
#let color-coral    = theme-accent
#let color-orange   = theme-primary
#let color-yellow   = rgb("#ffd166")
#let color-muted    = theme-muted
#let color-panel    = theme-panel
#let color-table-bd = theme-border
#let color-zebra    = rgb("#fdf3ec")

#let hard-shadow(inner, shadow: rgb("#3a1f18"), dx: px(6), dy: px(6), radius: px(16)) = context {
  let s = measure(inner)
  block(width: s.width + dx, height: s.height + dy)[
    #place(top + left, dx: dx, dy: dy,
      rect(width: s.width, height: s.height, fill: shadow, radius: radius))
    #place(top + left, inner)
  ]
}

#let h1-inner(body) = block(
  width: content-width - px(6),
  fill: gradient.linear(color-coral, color-orange, angle: 135deg),
  radius: px(20),
  inset: (x: px(22), y: px(18)),
)[
  #set text(font: hf(1, font-bold), size: tx(size-h1), weight: 800, fill: white)
  #set par(leading: 0.4em)
  #body
]
#let h1-block(body) = block(above: px(34), below: px(22),
  hard-shadow(h1-inner(body), shadow: rgb(255, 107, 139, 35%)))

#let h2-block(body) = block(
  width: 100%, above: px(34), below: px(18),
  fill: white,
  stroke: (paint: color-orange, thickness: px(2.5), dash: "dashed"),
  radius: px(14),
  inset: (left: px(16), right: px(14), y: px(12)),
)[
  #set text(font: hf(2, font-bold), size: tx(size-h2), weight: 700, fill: color-text)
  #set par(leading: 0.4em)
  #box(circle(radius: px(7), fill: color-coral))
  #h(px(10))
  #body
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  h2-block[CONTENTS]
  block(width: 100%, below: px(16))[
    #set par(leading: 0.6em)
    #stack(spacing: px(10), ..hs.enumerate().map(((i, h)) => grid(
      columns: (auto, 1fr), column-gutter: px(10), align: horizon,
      box(circle(radius: px(7),
        fill: (color-coral, color-orange, color-yellow).at(calc.rem(i, 3)))),
      link(h.location(), text(fill: color-soft, weight: 600, size: tx(16), h.body)),
    )))
  ]
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(above: px(36), width: 100%)[
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(14),
    align: (horizon, left + horizon, right + horizon),
    box(circle(radius: px(24), fill: gradient.linear(color-coral, color-orange, angle: 135deg))[
      #align(center + horizon, text(font: font-bold, size: tx(20), weight: 800, fill: white,
        if author != "" { author.first() } else { "" }))
    ]),
    [
      #text(font: font-text, size: tx(16), fill: color-text, weight: 700)[#author] \
      #text(font: font-text, size: tx(14), fill: color-muted)[#date]
    ],
    if logo != "" { image(logo, width: 52pt, height: 52pt) },
  )
]


#let divider() = block(width: 100%, above: px(36), below: px(36),
    line(length: 100%, stroke: (paint: color-orange, thickness: px(2), dash: "dashed")))

#let conf(doc) = {
  set page(
    ..if paper != "" { (paper: paper, flipped: p-orient == "landscape") } else {
      (width: long-page-width, height: if paged { long-page-min-height } else { auto })
    },
    margin: (
      left: margin-x * m-factor, right: margin-x * m-factor,
      top: margin-top * m-factor, bottom: margin-bot * m-factor,
    ),
    fill: color-bg,
    footer: if paged {
      context align(center)[
        #set text(size: tx(size-caption), fill: color-muted, tracking: 0pt)
        #if p-pagenum != "false" [
          #counter(page).display() / #counter(page).final().first()
        ]
        #if p-footer != "" [
          #if p-pagenum != "false" { linebreak() }
          #p-footer
        ]
      ]
    },
  )
  set text(font: font-text, fill: color-text, size: tx(size-body), lang: "zh", weight: 400, tracking: 0.02em)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.78em * l-factor,
    spacing: px(18),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => highlight(
    fill: color-yellow, top-edge: 0.32em, bottom-edge: -0.2em, extent: px(2), radius: px(3),
    text(weight: 700, fill: color-text, it.body),
  )
  show emph: it => text(style: "italic", fill: color-soft, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%, above: px(28), below: px(14),
    stroke: (left: px(4) + color-coral), inset: (left: px(12), y: 2pt),
  )[
    #set text(font: hf(3, font-bold), size: tx(size-h3), weight: 700, fill: color-text)
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(font: hf(4, font-text), size: tx(size-h4), weight: 700, fill: color-soft); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(font: hf(5, font-text), size: tx(size-h5), weight: 700, fill: color-muted); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(font: hf(6, font-text), size: tx(size-h6), weight: 700, fill: color-muted); #set par(leading: 0.4em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, { set image(width: w); it })
      })
    } else { it }
  }

  show link: it => text(fill: color-coral, weight: 600, it.body)

  show quote.where(block: true): it => block(width: 100%, above: px(28), below: px(28),
    hard-shadow(
      block(width: content-width - px(6), fill: white,
        stroke: px(2.5) + color-text, radius: px(14), inset: (x: px(20), y: px(16)))[
        #text(font: font-bold, size: tx(14), weight: 700, fill: color-coral, tracking: 0.18em)[QUOTE] \
        #v(px(8))
        #set text(size: tx(16), fill: color-text)
        #it.body
      ],
      shadow: color-text, radius: px(14),
    ))

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code), fill: color-coral)
  show raw.where(block: false): box.with(
    fill: rgb(255, 107, 139, 10%), inset: (x: px(5), y: px(1)), radius: px(4), outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-zebra,
    stroke: (paint: color-orange, thickness: px(2), dash: "dashed"),
    radius: px(10), inset: px(18),
  )[
    #set par(leading: 0.45em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: color-soft)
    #it
  ]

  set list(marker: text(fill: color-coral, weight: 700)[•], body-indent: px(8), spacing: px(8))
  set enum(
    numbering: n => box(
      width: px(22), height: px(22), radius: px(11),
      fill: gradient.linear(color-coral, color-orange, angle: 135deg),
      align(center + horizon, text(size: tx(13), weight: 800, fill: white, str(n))),
    ),
    body-indent: px(10), spacing: px(8),
  )

  set table(
    stroke: px(1.5) + color-table-bd, inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { rgb(255, 169, 77, 22%) } else if calc.even(row) { color-zebra } else { none },
  )
  show table: it => block(width: 100%, radius: px(10), clip: true,
    stroke: (paint: color-orange, thickness: px(2), dash: "dashed"), breakable: true, it)
  show table: set text(size: tx(15))
  show table.cell.where(y: 0): set text(weight: 700, fill: color-text)

  show math.equation: set text(font: ("New Computer Modern Math",))

  if paged or paper != "" {
    doc
  } else {
    context {
      let min-inner = long-page-min-height - margin-top - margin-bot
      let h = measure(doc).height
      if h < min-inner { block(height: min-inner, doc) } else { doc }
    }
  }
}