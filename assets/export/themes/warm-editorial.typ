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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#1c1a17"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#8a817a"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#faf7f2"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f3ece1"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e3d8c8"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#c0512f"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#9a3f24"))



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
#let content-width   = long-page-width - margin-x * 2
#let logo-url = ""

#let font-text  = (sys.inputs.at("theme-font-body", default: "Helvetica Neue"), "PingFang SC")
#let font-serif = (sys.inputs.at("theme-font-heading", default: "Songti SC"), "Songti SC", "Songti SC", "Helvetica Neue", "PingFang SC")
#let font-bold  = ("Songti SC", "Helvetica Neue", "PingFang SC", "PingFang SC")
#let font-mono  = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "DejaVu Sans Mono", "PingFang SC", "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg       = theme-bg
#let color-text     = theme-ink
#let color-soft     = rgb("#3d3933")
#let color-accent   = theme-primary
#let color-accent2  = theme-accent
#let color-muted    = theme-muted
#let color-line     = theme-ink
#let color-panel    = theme-panel
#let color-table-bd = theme-border
#let color-zebra    = rgb("#f5efe6")

#let masthead() = block(width: 100%, below: px(20))[
  #block(width: 100%, height: px(6), fill: color-accent)
  #v(px(10))
  #grid(
    columns: (1fr, auto), align: (left + horizon, right + horizon),
    text(font: font-serif, size: tx(13), fill: color-accent, weight: 600, tracking: 0.22em, style: "italic")[EDITORIAL],
    text(font: font-text, size: tx(12), fill: color-muted, tracking: 0.1em)[EDITORIAL NOTES · EXPERT EDITION],
  )
]

#let h1-block(body) = block(width: 100%, above: px(34), below: px(22))[
  #set text(font: hf(1, font-serif), size: tx(size-h1), weight: 600, fill: color-text)
  #set par(leading: 0.42em)
  #body
  #v(px(12))
  #block(width: px(56), height: px(3), fill: color-accent)
]

#let h2-block(body) = block(
  width: 100%, above: px(34), below: px(18),
  stroke: (left: px(3) + color-accent),
  inset: (left: px(16), right: px(4), top: px(2), bottom: px(2)),
)[
  #set text(font: hf(2, font-serif), size: tx(size-h2), weight: 600, fill: color-text)
  #set par(leading: 0.42em)
  #body
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(12), below: px(14))[
    #text(font: font-serif, size: tx(14), fill: color-accent, weight: 600, tracking: 0.22em, style: "italic")[CONTENTS]
  ]
  let romans = ("i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii")
  block(width: 100%, below: px(18))[
    #set par(leading: 0.6em)
    #stack(spacing: px(0), ..hs.enumerate().map(((i, h)) => {
      let r = if i < romans.len() { romans.at(i) } else { str(i + 1) }
      block(width: 100%, inset: (y: px(9)),
        stroke: (bottom: px(1) + rgb(28, 26, 23, 10%)))[
        #grid(
          columns: (px(34), 1fr), column-gutter: px(12), align: (left + horizon, left + horizon),
          text(font: font-serif, style: "italic", fill: color-accent, weight: 600, size: tx(16))[#(r).],
          link(h.location(), text(font: font-serif, fill: color-soft, weight: 500, size: tx(16), h.body)),
        )
      ]
    }))
  ]
}

#let poster-title(title) = { masthead(); h1-block(title) }

#let signature-row(author: "", date: "", logo: logo-url) = block(above: px(36), width: 100%)[
  #block(width: 100%, height: px(2), fill: rgb(28, 26, 23, 10%))
  #v(px(16))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(14),
    align: (horizon, left + horizon, right + horizon),
    box(width: px(46), height: px(46), radius: 50%,
      stroke: px(1.5) + color-accent, fill: color-panel)[
      #align(center + horizon, text(font: font-serif, size: tx(22), weight: 600, fill: color-accent,
        if author != "" { author.first() } else { "" }))
    ],
    [
      #text(font: font-serif, size: tx(16), fill: color-text, weight: 600)[#author] \
      #text(font: font-text, size: tx(13), fill: color-muted, tracking: 0.06em)[#date]
    ],
    if logo != "" { image(logo, width: 52pt, height: 52pt) },
  )
]


#let divider() = align(center, block(above: px(38), below: px(38))[
    #grid(columns: (px(40), auto, px(40)), column-gutter: px(12), align: horizon,
      line(length: 100%, stroke: px(1) + rgb(28, 26, 23, 22%)),
      rotate(45deg, rect(width: px(7), height: px(7), fill: color-accent)),
      line(length: 100%, stroke: px(1) + rgb(28, 26, 23, 22%)),
    )
  ])

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
  set text(font: font-text, fill: color-text, size: tx(size-body), lang: "zh", weight: 400, tracking: 0.01em)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.82em * l-factor,
    spacing: px(18),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => text(weight: 700, fill: color-accent2, it.body)
  show emph: it => text(style: "italic", font: font-serif, fill: color-soft, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%, above: px(28), below: px(14),
  )[
    #set text(font: hf(3, font-serif), size: tx(size-h3), weight: 600, fill: color-text)
    #set par(leading: 0.4em)
    #box(baseline: -0.05em, rect(width: px(7), height: px(7), fill: color-accent))
    #h(px(9))
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(font: hf(4, font-serif), size: tx(size-h4), weight: 600, fill: color-soft); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(font: hf(5, font-serif), size: tx(size-h5), weight: 600, fill: color-muted); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(font: hf(6, font-serif), size: tx(size-h6), weight: 600, fill: color-muted, tracking: 0.1em); #set par(leading: 0.4em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, { set image(width: w); it })
      })
    } else { it }
  }

  show link: it => text(fill: color-accent, weight: 600, it.body)

  show quote.where(block: true): it => block(
    width: 100%, above: px(28), below: px(28),
    fill: color-panel,
    stroke: (left: px(4) + color-accent),
    inset: (left: px(22), right: px(20), top: px(16), bottom: px(18)),
  )[
    #place(top + left, dx: px(-4), dy: px(-14),
      text(font: font-serif, size: tx(58), fill: rgb(192, 81, 47, 22%), weight: 600, [“]))
    #set text(font: font-serif, size: tx(17), fill: color-soft, style: "italic")
    #set par(leading: 0.6em)
    #it.body
  ]

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code), fill: color-accent2)
  show raw.where(block: false): box.with(
    fill: rgb(192, 81, 47, 8%), inset: (x: px(5), y: px(1)), radius: px(3), outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-zebra,
    stroke: (left: px(3) + color-accent),
    inset: px(18),
  )[
    #set par(leading: 0.5em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: color-soft)
    #it
  ]

  set list(marker: text(fill: color-accent, weight: 700)[—], body-indent: px(8), spacing: px(9))
  set enum(
    numbering: n => box(width: px(26),
      text(font: font-serif, style: "italic", size: tx(16), weight: 600, fill: color-accent)[#numbering("i.", n)]),
    body-indent: px(8), spacing: px(10),
  )

  set table(
    stroke: none, inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-panel } else if calc.even(row) { color-zebra } else { none },
  )
  show table: it => block(width: 100%, breakable: true,
    stroke: (top: px(2) + color-accent, bottom: px(1.5) + color-table-bd), it)
  show table: set text(size: tx(15))
  show table.cell.where(y: 0): set text(font: font-serif, weight: 700, fill: color-text)

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