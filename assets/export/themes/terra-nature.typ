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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#2e3230"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#4a4e4a"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#faf6f0"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f5f1ea"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#d8d5cc"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#4a7c59"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#705c30"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "30"))
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

#let logo-url = ""

#let font-text  = (sys.inputs.at("theme-font-body", default: "Helvetica Neue"), "PingFang SC")
#let font-bold  = ("Helvetica Neue", "PingFang SC", "PingFang SC")
#let font-serif = (sys.inputs.at("theme-font-heading", default: "Songti SC"), "PingFang SC")
#let font-mono  = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "DejaVu Sans Mono", "PingFang SC", "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg     = theme-bg
#let color-bg2    = rgb("#e9f1ea")
#let color-green   = theme-primary
#let color-green-hi = rgb("#2a6038")
#let color-amber  = theme-accent
#let color-ink    = theme-ink
#let color-soft   = theme-muted
#let color-muted  = rgb(74, 78, 74, 60%)
#let color-card   = rgb(255, 255, 255, 70%)
#let color-accent-bg = rgb(74, 124, 89, 4%)
#let color-accent-bd = rgb(74, 124, 89, 16%)
#let color-hair   = rgb(116, 121, 110, 16%)
#let color-line   = rgb(116, 121, 110, 10%)
#let color-chip   = rgb(74, 124, 89, 11%)
#let color-chip2  = rgb(74, 124, 89, 16%)
#let color-zebra  = theme-panel
#let color-panel  = rgb("#f0ece4")
#let color-table-bd = rgb(116, 121, 110, 16%)

#let hairline(above: px(0), below: px(0)) = block(width: 100%, above: above, below: below,
  line(length: 100%, stroke: px(1) + color-hair))

#let h1-block(body) = block(width: 100%, above: px(34), below: px(20))[
  #box(width: px(28), height: px(5), radius: px(3), fill: color-green)
  #v(px(10))
  #set text(font: hf(1, font-serif), size: tx(size-h1), weight: 700, tracking: -0.01em,
    fill: gradient.linear(color-green, color-green-hi, angle: 20deg))
  #set par(leading: 0.42em)
  #body
  #v(px(14))
  #line(length: 100%, stroke: px(1) + color-hair)
]

#let h2-block(body) = block(width: 100%, above: px(32), below: px(16))[
  #grid(columns: (auto, 1fr), column-gutter: px(12), align: horizon,
    box(width: px(24), height: px(5), radius: px(3), fill: color-green),
    {
      set text(font: hf(2, font-serif), size: tx(size-h2), weight: 700, fill: color-ink)
      set par(leading: 0.42em)
      body
    },
  )
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(20), below: px(8))[
    #text(font: font-text, size: tx(13), weight: 600, fill: color-green, tracking: 0.16em)[CONTENTS · CONTENTS]
  ]
  block(width: 100%, below: px(16),
    fill: color-card, stroke: px(1) + color-line, radius: px(16), inset: px(18))[
    #set par(leading: 0.6em)
    #stack(spacing: px(12), ..hs.map(h => grid(
      columns: (auto, 1fr), column-gutter: px(12), align: horizon,
      box(width: px(15), height: px(15), radius: px(5),
        stroke: px(2) + color-accent-bd),
      link(h.location(), text(fill: color-soft, weight: 500, size: tx(16), h.body)),
    )))
  ]
  hairline(above: px(18), below: px(0))
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(above: px(34), width: 100%)[
  #hairline(below: px(16))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(12),
    align: (horizon, left + horizon, right + horizon),
    box(circle(radius: px(20), fill: color-chip, stroke: px(1) + color-accent-bd)[
      #align(center + horizon, text(font: font-serif, size: tx(18), weight: 700, fill: color-green,
        if author != "" { author.first() } else { "" }))
    ]),
    [
      #text(font: font-text, size: tx(16), fill: color-ink, weight: 600)[#author] \
      #text(font: font-text, size: tx(14), fill: color-muted)[#date]
    ],
    if logo != "" { image(logo, width: 50pt, height: 50pt) },
  )
]


#let divider() = block(width: 100%, above: px(34), below: px(34),
    line(length: 100%, stroke: px(1) + gradient.linear(
      rgb(116, 121, 110, 0%), color-hair, rgb(116, 121, 110, 0%))))

#let conf(doc) = {
  set page(
    ..if paper != "" { (paper: paper, flipped: p-orient == "landscape") } else {
      (width: long-page-width, height: if paged { long-page-min-height } else { auto })
    },
    margin: (
      left: margin-x * m-factor, right: margin-x * m-factor,
      top: margin-top * m-factor, bottom: margin-bot * m-factor,
    ),
    fill: gradient.radial(color-bg2, color-bg, center: (88%, 4%), radius: 115%),
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
  set text(font: font-text, fill: color-ink, size: tx(size-body), lang: "zh", weight: 400, tracking: 0.01em)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.82em * l-factor,
    spacing: px(18),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => box(
    fill: color-chip, inset: (x: px(5), y: px(1)), radius: px(5), outset: (y: px(2)),
    text(weight: 700, fill: color-green, it.body),
  )
  show emph: it => text(style: "italic", fill: color-amber, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(width: 100%, above: px(28), below: px(12))[
    #set text(font: hf(3, font-serif), size: tx(size-h3), weight: 700, fill: color-green)
    #set par(leading: 0.42em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(font: hf(4, font-text), size: tx(size-h4), weight: 600, fill: color-ink); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(font: hf(5, font-text), size: tx(size-h5), weight: 600, fill: color-soft); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(font: hf(6, font-text), size: tx(size-h6), weight: 600, fill: color-muted); #set par(leading: 0.42em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, block(radius: px(16), clip: true, stroke: px(1) + color-line,
          { set image(width: w); it }))
      })
    } else { it }
  }

  show link: it => text(fill: color-green, weight: 500, it.body)

  show quote.where(block: true): it => block(
    width: 100%, above: px(26), below: px(26),
    fill: color-accent-bg,
    stroke: (left: px(3) + color-green, rest: px(1) + color-line),
    radius: (right: px(14)),
    inset: (left: px(22), rest: px(18)),
  )[
    #grid(columns: (auto, 1fr), column-gutter: px(8), align: horizon,
      text(font: font-serif, size: tx(34), weight: 600, fill: color-green, tracking: 0em)[“],
      text(font: font-serif, size: tx(11), weight: 600, fill: color-green, tracking: 0.16em)[QUOTE],
    )
    #v(px(6))
    #set text(font: font-serif, size: tx(16), fill: color-ink, style: "italic")
    #it.body
  ]

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code), fill: color-green)
  show raw.where(block: false): box.with(
    fill: color-chip, inset: (x: px(5), y: px(1)), radius: px(5), outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-panel,
    stroke: px(1) + color-accent-bd,
    radius: px(16), inset: px(18),
  )[
    #set par(leading: 0.5em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: color-soft)
    #it
  ]

  set list(marker: text(fill: color-green, weight: 700)[•], body-indent: px(8), spacing: px(8))
  set enum(
    numbering: n => box(
      width: px(22), height: px(22), radius: px(11), fill: color-chip,
      align(center + horizon, text(size: tx(13), weight: 700, fill: color-green, str(n))),
    ),
    body-indent: px(10), spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd, inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-chip2 } else if calc.even(row) { color-zebra } else { none },
  )
  show table: it => block(width: 100%, radius: px(14), clip: true,
    stroke: px(1) + color-table-bd, breakable: true, it)
  show table: set text(size: tx(15), fill: color-soft)
  show table.cell.where(y: 0): set text(weight: 700, fill: color-green)

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
