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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#191c1d"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#5a6570"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#f7f9fb"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#eef1f4"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#d5dbe1"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#12161a"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#12161a"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "30"))
#let size-h2      = float(sys.inputs.at("theme-size-h2",      default: "23"))
#let size-h3      = float(sys.inputs.at("theme-size-h3",      default: "21"))
#let size-h4      = float(sys.inputs.at("theme-size-h4",      default: "17"))
#let size-h5      = float(sys.inputs.at("theme-size-h5",      default: "15"))
#let size-h6      = float(sys.inputs.at("theme-size-h6",      default: "14"))
#let size-body    = float(sys.inputs.at("theme-size-body",    default: "15"))
#let size-code    = float(sys.inputs.at("theme-size-code",    default: "14"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let long-page-width      = 540pt
#let margin-x        = 39pt
#let margin-top      = 60pt
#let margin-bot      = 48pt
#let long-page-min-height = 720pt

#let logo-url = ""

#let font-text  = (sys.inputs.at("theme-font-body", default: "Helvetica Neue"), "PingFang SC")
#let font-bold  = (sys.inputs.at("theme-font-heading", default: "Helvetica Neue"), "PingFang SC", "PingFang SC")
#let font-mono  = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "DejaVu Sans Mono", "PingFang SC", "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg      = rgb("#f1fbff")
#let color-panel   = rgb("#eaf5fa")
#let color-panel2  = rgb("#dfeaef")
#let color-ink     = rgb("#181f21")
#let color-text    = rgb("#131d21")
#let color-soft    = rgb("#434749")
#let color-muted   = rgb(67, 71, 73, 70%)
#let color-faint   = rgb("#747879")
#let color-line    = rgb("#c3c7c8")
#let color-hair    = rgb("#dfeaef")
#let color-code-bg = rgb("#283236")
#let color-code-fg = rgb("#e7f3f7")
#let color-zebra   = rgb("#eaf5fa")
#let color-table-bd = rgb("#c3c7c8")

#let hairline(above: px(0), below: px(0)) = block(width: 100%, above: above, below: below,
  line(length: 100%, stroke: px(1) + color-line))

#let h1-block(body) = block(width: 100%, above: px(34), below: px(20))[
  #box(stroke: px(1) + color-line, radius: px(3), inset: (x: px(7), y: px(3)),
    text(font: font-text, size: tx(11), weight: 700, fill: color-soft, tracking: 0.18em,
      "CONFIDENTIAL"))
  #v(px(12))
  #set text(font: hf(1, font-bold), size: tx(size-h1), weight: 700, tracking: -0.01em, fill: color-ink)
  #set par(leading: 0.42em)
  #body
  #v(px(14))
  #line(length: 100%, stroke: px(2) + color-hair)
]

#let h2-block(body) = block(width: 100%, above: px(32), below: px(16))[
  #grid(columns: (auto, 1fr), column-gutter: px(12), align: horizon,
    box(width: px(24), height: px(5), radius: px(2), fill: color-ink),
    {
      set text(font: hf(2, font-bold), size: tx(size-h2), weight: 700, fill: color-ink)
      set par(leading: 0.42em)
      body
    },
  )
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(20), below: px(8))[
    #text(font: font-text, size: tx(12), weight: 700, fill: color-soft, tracking: 0.18em,
      "CONTENTS · CONTENTS")
  ]
  block(width: 100%, below: px(16),
    fill: color-panel, stroke: px(1) + color-line, radius: px(8), inset: px(18))[
    #set par(leading: 0.6em)
    #stack(spacing: px(12), ..hs.map(h => grid(
      columns: (auto, 1fr), column-gutter: px(12), align: horizon,
      box(width: px(15), height: px(15), radius: px(2),
        stroke: px(2) + color-faint),
      link(h.location(), text(fill: color-text, weight: 500, size: tx(16), h.body)),
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
    box(width: px(40), height: px(40), radius: px(4), fill: color-panel,
      stroke: px(1) + color-line)[
      #align(center + horizon, text(font: font-bold, size: tx(18), weight: 700, fill: color-ink,
        if author != "" { author.first() } else { "" }))
    ],
    [
      #text(font: font-text, size: tx(16), fill: color-ink, weight: 600)[#author] \
      #text(font: font-mono, size: tx(13), fill: color-muted, tracking: 0.04em)[#date]
    ],
    if logo != "" { image(logo, width: 50pt, height: 50pt) },
  )
]


#let divider() = block(width: 100%, above: px(34), below: px(34),
    line(length: 100%, stroke: px(1) + color-hair))

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
        #set text(font: font-mono, size: tx(size-caption), fill: color-muted, tracking: 0.04em)
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
  set text(font: font-text, fill: color-text, size: tx(size-body), lang: "zh", weight: 400, tracking: 0.01em)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.82em * l-factor,
    spacing: px(18),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => box(
    fill: color-panel2, inset: (x: px(5), y: px(1)), radius: px(3), outset: (y: px(2)),
    text(weight: 700, fill: color-ink, it.body),
  )
  show emph: it => text(style: "italic", fill: color-soft, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(width: 100%, above: px(28), below: px(12))[
    #set text(font: hf(3, font-bold), size: tx(size-h3), weight: 700, fill: color-ink)
    #set par(leading: 0.42em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(font: hf(4, font-text), size: tx(size-h4), weight: 600, fill: color-soft); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(font: hf(5, font-text), size: tx(size-h5), weight: 600, fill: color-muted); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(font: hf(6, font-text), size: tx(size-h6), weight: 600, fill: color-muted); #set par(leading: 0.42em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, block(radius: px(8), clip: true, stroke: px(1) + color-line,
          { set image(width: w); it }))
      })
    } else { it }
  }

  show link: it => text(fill: color-ink, weight: 600, it.body)

  show quote.where(block: true): it => block(
    width: 100%, above: px(26), below: px(26),
    fill: color-panel,
    stroke: (left: px(4) + color-ink, rest: px(1) + color-line),
    radius: (right: px(8)),
    inset: (left: px(22), rest: px(18)),
  )[
    #text(font: font-mono, size: tx(11), weight: 700, fill: color-soft, tracking: 0.16em,
      "MEMO · EXCERPT")
    #v(px(8))
    #set text(size: tx(16), fill: color-ink, style: "italic")
    #it.body
  ]

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code), fill: color-ink)
  show raw.where(block: false): box.with(
    fill: color-panel2, inset: (x: px(5), y: px(1)), radius: px(3), outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-code-bg,
    radius: px(8), inset: px(18),
  )[
    #set par(leading: 0.5em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: color-code-fg)
    #it
  ]

  set list(marker: text(fill: color-ink, weight: 700)[•], body-indent: px(8), spacing: px(8))
  set enum(
    numbering: n => box(
      width: px(22), height: px(22), radius: px(2), fill: color-panel2,
      align(center + horizon, text(font: font-mono, size: tx(13), weight: 700, fill: color-ink, str(n))),
    ),
    body-indent: px(10), spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd, inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-panel2 } else if calc.even(row) { color-zebra } else { none },
  )
  show table: it => block(width: 100%, radius: px(8), clip: true,
    stroke: px(1) + color-table-bd, breakable: true, it)
  show table: set text(size: tx(15), fill: color-text)
  show table.cell.where(y: 0): set text(weight: 700, fill: color-ink)

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