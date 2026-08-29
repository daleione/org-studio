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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#271717"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#5b403f"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#fff8f7"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#ffe9e8"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e4bebc"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#b7102a"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#b7102a"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "34"))
#let size-h2      = float(sys.inputs.at("theme-size-h2",      default: "22"))
#let size-h3      = float(sys.inputs.at("theme-size-h3",      default: "21"))
#let size-h4      = float(sys.inputs.at("theme-size-h4",      default: "17"))
#let size-h5      = float(sys.inputs.at("theme-size-h5",      default: "15"))
#let size-h6      = float(sys.inputs.at("theme-size-h6",      default: "13"))
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

#let color-bg     = theme-bg
#let color-panel  = theme-panel
#let color-panel2 = rgb("#fff0ef")
#let color-card   = rgb("#ffffff")
#let color-red    = theme-primary
#let color-red-hi = rgb("#db313f")
#let color-blue   = rgb("#485f84")
#let color-blue-bg = rgb("#bbd3fd")
#let color-yellow = rgb("#e8c200")
#let color-ink    = theme-ink
#let color-text   = theme-ink
#let color-soft   = theme-muted
#let color-muted  = rgb("#8f6f6e")
#let color-hair   = theme-border
#let color-zebra  = rgb("#fff0ef")
#let color-chip   = rgb("#ffe1e0")

#let heavy-rule(above: px(0), below: px(0)) = block(width: 100%, above: above, below: below,
  line(length: 100%, stroke: px(3) + color-ink))
#let hairline(above: px(0), below: px(0)) = block(width: 100%, above: above, below: below,
  line(length: 100%, stroke: px(1) + color-hair))

#let h1-block(body) = block(width: 100%, above: px(34), below: px(20))[
  #set text(font: hf(1, font-bold), size: tx(size-h1), weight: 800, tracking: -0.02em, fill: color-ink)
  #set par(leading: 0.34em)
  #body
  #v(px(16))
  #line(length: 100%, stroke: px(3) + color-ink)
]

#let h2-block(body) = block(width: 100%, above: px(34), below: px(18))[
  #grid(columns: (auto, 1fr), column-gutter: px(10), align: horizon,
    box(width: px(9), height: px(34), fill: color-red),
    box(fill: color-red, inset: (x: px(12), y: px(7)))[
      #set text(font: hf(2, font-bold), size: tx(size-h2), weight: 800, fill: white, tracking: 0.02em)
      #set par(leading: 0.36em)
      #body
    ],
  )
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(20), below: px(8))[
    #text(font: font-text, size: tx(13), weight: 700, fill: color-red, tracking: 0.2em)[CONTENTS · CONTENTS]
  ]
  block(width: 100%, below: px(16),
    fill: color-card, stroke: px(3) + color-ink, inset: px(18))[
    #set par(leading: 0.6em)
    #stack(spacing: px(12), ..hs.map(h => grid(
      columns: (auto, 1fr), column-gutter: px(12), align: horizon,
      box(width: px(11), height: px(11), fill: color-red),
      link(h.location(), text(fill: color-text, weight: 600, size: tx(16), h.body)),
    )))
  ]
  heavy-rule(above: px(18), below: px(0))
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(above: px(34), width: 100%)[
  #heavy-rule(below: px(16))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(12),
    align: (horizon, left + horizon, right + horizon),
    box(fill: color-red, width: px(42), height: px(42))[
      #align(center + horizon, text(font: font-bold, size: tx(18), weight: 800, fill: white,
        if author != "" { author.first() } else { "" }))
    ],
    [
      #text(font: font-text, size: tx(16), fill: color-ink, weight: 700)[#author] \
      #text(font: font-mono, size: tx(13), fill: color-muted, tracking: 0.08em)[#date]
    ],
    if logo != "" { image(logo, width: 50pt, height: 50pt) },
  )
]


#let divider() = block(width: 100%, above: px(34), below: px(34))[
    #grid(columns: (auto, 1fr), column-gutter: px(0), align: horizon,
      box(width: px(14), height: px(14), fill: color-red),
      line(length: 100%, stroke: px(3) + color-ink),
    )
  ]

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
        #set text(font: font-mono, size: tx(size-caption), fill: color-muted, tracking: 0.1em)
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
  set text(font: font-text, fill: color-text, size: tx(size-body), lang: "zh", weight: 400, tracking: 0pt)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.82em * l-factor,
    spacing: px(18),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => box(
    fill: color-chip, inset: (x: px(5), y: px(1)), radius: px(0), outset: (y: px(2)),
    text(weight: 800, fill: color-red, it.body),
  )
  show emph: it => text(style: "italic", fill: color-blue, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(width: 100%, above: px(28), below: px(12))[
    #set text(font: hf(3, font-bold), size: tx(size-h3), weight: 800, fill: color-ink, tracking: -0.01em)
    #set par(leading: 0.40em)
    #grid(columns: (auto, 1fr), column-gutter: px(9), align: horizon,
      box(width: px(7), height: px(18), fill: color-ink),
      it.body,
    )
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(font: hf(4, font-bold), size: tx(size-h4), weight: 700, fill: color-soft); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(font: hf(5, font-text), size: tx(size-h5), weight: 700, fill: color-muted); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(font: hf(6, font-mono), size: tx(size-h6), weight: 700, fill: color-muted, tracking: 0.1em); #set par(leading: 0.42em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, block(fill: color-card, stroke: px(3) + color-ink, inset: px(6), clip: false,
          { set image(width: w); it }))
      })
    } else { it }
  }

  show link: it => text(fill: color-red, weight: 700, it.body)

  show quote.where(block: true): it => block(
    width: 100%, above: px(26), below: px(26),
    fill: color-chip,
    stroke: (left: px(6) + color-red, rest: px(1) + color-hair),
    inset: (left: px(22), rest: px(18)),
  )[
    #grid(columns: (auto, 1fr), column-gutter: px(8), align: horizon,
      text(font: font-bold, size: tx(34), weight: 800, fill: color-red, tracking: 0em)[“],
      text(font: font-mono, size: tx(11), weight: 700, fill: color-red, tracking: 0.2em)[QUOTE],
    )
    #v(px(6))
    #set text(size: tx(16), fill: color-ink, style: "italic")
    #it.body
  ]

  show raw.where(block: false): set text(font: font-mono, size: tx(13), fill: color-red)
  show raw.where(block: false): box.with(
    fill: color-chip, inset: (x: px(5), y: px(1)), radius: px(0), outset: (y: px(2)),
    stroke: px(1) + color-ink,
  )
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-card,
    stroke: px(3) + color-ink,
    inset: px(18),
  )[
    #text(font: font-mono, size: tx(11), weight: 700, fill: color-red, tracking: 0.2em, "// BUILD LOGIC")
    #v(px(10))
    #block(width: 100%, fill: color-ink, inset: px(14))[
      #set par(leading: 0.5em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: rgb("#ffedeb"))
      #it
    ]
  ]

  set list(marker: box(width: px(8), height: px(8), fill: color-red), body-indent: px(10), spacing: px(8))
  set enum(
    numbering: n => box(
      width: px(22), height: px(22), fill: color-ink,
      align(center + horizon, text(font: font-mono, size: tx(13), weight: 700, fill: white, str(n))),
    ),
    body-indent: px(10), spacing: px(8),
  )

  set table(
    stroke: px(1) + color-ink, inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-red } else if calc.even(row) { color-zebra } else { color-card },
  )
  show table: it => block(width: 100%, stroke: px(3) + color-ink, clip: true, breakable: true, it)
  show table: set text(font: font-mono, size: tx(14), fill: color-text)
  show table.cell.where(y: 0): set text(weight: 700, fill: white)

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
