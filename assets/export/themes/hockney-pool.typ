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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#123a3a"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#4a6b68"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#fdf6e3"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f7efd8"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e5dcc0"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#0f6b6b"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#ffd400"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "32"))
#let size-h2      = float(sys.inputs.at("theme-size-h2",      default: "24"))
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

#let font-text   = (sys.inputs.at("theme-font-body", default: "Helvetica Neue"), "PingFang SC")
#let font-bold   = ("Helvetica Neue", "PingFang SC", "PingFang SC")
#let font-serif  = (sys.inputs.at("theme-font-heading", default: "Songti SC"), "PingFang SC")
#let font-mono   = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "DejaVu Sans Mono", "PingFang SC", "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg      = rgb("#fdf9f3")
#let color-card    = rgb("#f1ede7")
#let color-cardlow = rgb("#f7f3ed")   // surface-container-low
#let color-teal    = rgb("#00677f")
#let color-teal-hi = rgb("#00a9ce")
#let color-teal-dk = rgb("#003846")
#let color-yellow  = rgb("#fcd400")
#let color-yellow2 = rgb("#e9c400")
#let color-olive   = rgb("#705d00")
#let color-coral   = rgb("#cf8798")
#let color-ink     = rgb("#1c1c18")
#let color-soft    = rgb("#3d494d")
#let color-muted   = rgb(61, 73, 77, 60%)
#let color-line    = rgb("#bcc8ce")
#let color-hair    = rgb(109, 121, 126, 28%)
#let color-tealtint = rgb(0, 103, 127, 8%)
#let color-yelltint = rgb(252, 212, 0, 26%)

#let hairline(above: px(0), below: px(0)) = block(width: 100%, above: above, below: below,
  line(length: 100%, stroke: px(1) + color-hair))

#let h1-block(body) = block(width: 100%, above: px(34), below: px(20))[
  #box(width: px(56), height: px(8), radius: px(2), fill: color-yellow)
  #v(px(12))
  #set text(font: hf(1, font-serif), size: tx(size-h1), weight: 700, tracking: -0.01em, fill: color-teal-dk)
  #set par(leading: 0.42em)
  #body
  #v(px(14))
  #line(length: 100%, stroke: px(3) + color-teal-hi)
]

#let h2-block(body) = block(width: 100%, above: px(34), below: px(16))[
  #grid(columns: (auto, 1fr), column-gutter: px(12), align: horizon,
    box(width: px(8), height: px(26), radius: px(2), fill: color-yellow),
    {
      set text(font: hf(2, font-bold), size: tx(size-h2), weight: 700, fill: color-teal)
      set par(leading: 0.42em)
      box(inset: (bottom: px(4)), stroke: (bottom: px(3) + color-teal-hi), body)
    },
  )
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(22), below: px(10))[
    #text(font: font-text, size: tx(13), weight: 700, fill: color-teal, tracking: 0.18em)[CONTENTS · CONTENTS]
  ]
  block(width: 100%, below: px(18))[
    #box(width: 100%)[
      #place(dx: px(7), dy: px(7), block(width: 100%, height: 100%, radius: px(12), fill: color-teal))
      #block(width: 100%, fill: color-yellow, radius: px(12), inset: px(20))[
        #set par(leading: 0.6em)
        #stack(spacing: px(13), ..hs.map(h => grid(
          columns: (auto, 1fr), column-gutter: px(12), align: horizon,
          box(width: px(15), height: px(15), radius: px(4),
            stroke: px(2) + color-teal-dk),
          link(h.location(), text(fill: color-teal-dk, weight: 600, size: tx(16), h.body)),
        )))
      ]
    ]
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
    box(circle(radius: px(20), fill: color-yellow, stroke: px(2) + color-teal)[
      #align(center + horizon, text(font: font-bold, size: tx(18), weight: 700, fill: color-teal-dk,
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
    grid(columns: (px(40), px(40), 1fr), rows: px(4), column-gutter: px(6),
      box(fill: color-teal, width: 100%, height: 100%, radius: px(2)),
      box(fill: color-yellow, width: 100%, height: 100%, radius: px(2)),
      box(fill: color-line, width: 100%, height: 100%, radius: px(2))))

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
  set text(font: font-text, fill: color-ink, size: tx(size-body), lang: "zh", weight: 400, tracking: 0.01em)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.82em * l-factor,
    spacing: px(18),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => box(
    fill: color-yelltint, inset: (x: px(5), y: px(1)), radius: px(3), outset: (y: px(2)),
    text(weight: 700, fill: color-teal-dk, it.body),
  )
  show emph: it => text(style: "italic", fill: color-teal, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(width: 100%, above: px(28), below: px(12))[
    #set text(font: hf(3, font-bold), size: tx(size-h3), weight: 700, fill: color-teal-hi)
    #set par(leading: 0.42em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(font: hf(4, font-text), size: tx(size-h4), weight: 600, fill: color-teal); #set par(leading: 0.42em); #it.body
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
        align(center, block(radius: px(12), clip: true, stroke: px(2) + color-coral,
          { set image(width: w); it }))
      })
    } else { it }
  }

  show link: it => text(fill: color-teal, weight: 600, it.body)

  show quote.where(block: true): it => block(
    width: 100%, above: px(26), below: px(26),
    fill: color-card,
    stroke: (left: px(6) + color-teal-hi, rest: none),
    radius: (right: px(10)),
    inset: (left: px(22), rest: px(18)),
  )[
    #text(font: font-serif, size: tx(28), weight: 700, fill: color-teal-hi)[“]
    #v(px(2))
    #set text(font: font-serif, size: tx(17), fill: color-teal-dk, style: "italic", weight: 500)
    #it.body
  ]

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code), fill: color-teal)
  show raw.where(block: false): box.with(
    fill: color-tealtint, inset: (x: px(5), y: px(1)), radius: px(4), outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-ink,
    stroke: px(2) + color-soft,
    radius: px(12), inset: px(18),
  )[
    #set par(leading: 0.5em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: rgb("#57d5fc"))
    #it
  ]

  set list(marker: text(fill: color-teal-hi, weight: 700)[•], body-indent: px(8), spacing: px(8))
  set enum(
    numbering: n => box(
      width: px(22), height: px(22), radius: px(11), fill: color-yellow,
      align(center + horizon, text(size: tx(13), weight: 700, fill: color-teal-dk, str(n))),
    ),
    body-indent: px(10), spacing: px(8),
  )

  set table(
    stroke: px(1) + color-line, inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-teal-hi } else if calc.even(row) { color-cardlow } else { none },
  )
  show table: it => block(width: 100%, radius: px(10), clip: true,
    stroke: px(2) + color-line, breakable: true, it)
  show table: set text(size: tx(15), fill: color-soft)
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