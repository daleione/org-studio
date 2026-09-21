// ═══════════════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════════════

#let paged = sys.inputs.at("paged", default: "false") == "true"
#let paper = sys.inputs.at("paper", default: "")

#let p-fscale = float(sys.inputs.at("font-scale", default: "1.0"))
#let p-layout-scale = float(sys.inputs.at("layout-scale", default: "1.0"))
#let p-orient = sys.inputs.at("orientation", default: "")      // portrait|landscape
#let p-indent = sys.inputs.at("indent", default: "")
#let p-justify = sys.inputs.at("justify", default: "")
#let p-pagenum = sys.inputs.at("page-number", default: "")
#let p-footer = sys.inputs.at("footer-text", default: "")

#let m-factor = float(sys.inputs.at("margin-scale", default: "1.0"))
#let l-factor = float(sys.inputs.at("line-height-scale", default: "1.0"))

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#e2e2e2"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#e7bdb2"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#131313"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#1b1b1b"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#353535"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#ffb5a0",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#ff5625"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "32"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "23"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "20"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "17"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "15"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "13"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "15"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "14"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let long-page-width = float(sys.inputs.at("long-page-width-pt", default: "540")) * 1pt
#let margin-x = float(sys.inputs.at("long-page-margin-x-pt", default: "39")) * 1pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt

#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "DIN Alternate"),
  "Hiragino Sans GB",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "DIN Alternate"),
  "Hiragino Sans GB",
)
#let font-mono = (
  sys.inputs.at("theme-font-mono", default: "Menlo"),
  "DejaVu Sans Mono",
  "PingFang SC",
  "Apple Color Emoji",
)
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg = theme-bg
#let color-bg2 = rgb("#1a1413")
#let color-peach = theme-primary
#let color-peach-hi = rgb("#ffdbd1")
#let color-volt = theme-accent
#let color-ink = theme-ink
#let color-soft = theme-muted
#let color-muted = rgb(231, 189, 178, 60%)
#let color-panel = theme-panel
#let color-panel2 = rgb("#0e0e0e")
#let color-panel3 = rgb("#2a2a2a")
#let color-hair = theme-border
#let color-line = rgb("#1f1f1f")
#let color-chip = rgb(255, 86, 37, 14%)
#let color-zebra = rgb(255, 255, 255, 2%)
#let color-table-bd = theme-border

#let hairline(above: px(0), below: px(0)) = block(
  width: 100%,
  above: above,
  below: below,
  line(length: 100%, stroke: px(1) + color-hair),
)

#let h1-block(body) = block(width: 100%, above: px(34), below: px(20))[
  #set text(
    font: hf(1, font-bold),
    size: tx(size-h1),
    weight: 800,
    tracking: 0.07em,
    fill: gradient.linear(color-peach, color-peach-hi, angle: 18deg),
  )
  #set par(leading: 0.40em, justify: false)
  #body
  #v(px(14))
  #line(length: 100%, stroke: px(2) + color-volt)
]

#let h2-block(body) = block(width: 100%, above: px(34), below: px(18))[
  #grid(
    columns: (auto, 1fr),
    column-gutter: px(12),
    align: horizon,
    box(width: px(8), height: px(22), fill: color-volt),
    {
      set text(
        font: hf(2, font-bold),
        size: tx(size-h2),
        weight: 800,
        fill: color-peach,
        tracking: 0.02em,
      )
      set par(leading: 0.40em)
      body
    },
  )
  #v(px(10))
  #line(length: 100%, stroke: px(1) + color-hair)
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(20), below: px(8))[
    #text(
      font: font-mono,
      size: tx(12),
      weight: 500,
      fill: color-volt,
      tracking: 0.2em,
      "INDEX // CONTENTS",
    )
  ]
  block(
    width: 100%,
    below: px(16),
    fill: color-panel,
    stroke: px(1) + color-hair,
    inset: px(18),
  )[
    #set par(leading: 0.6em)
    #stack(spacing: px(12), ..hs.map(h => grid(
      columns: (auto, 1fr),
      column-gutter: px(12),
      align: horizon,
      box(
        width: px(13),
        height: px(13),
        fill: none,
        stroke: px(2) + color-volt,
      ),
      link(h.location(), text(
        fill: color-soft,
        weight: 500,
        size: tx(16),
        h.body,
      )),
    )))
  ]
  hairline(above: px(18), below: px(0))
}

// A voltage label, not a document title: the band bleeds to the page edges and
// the title is knocked out of the orange in near-black.
#let poster-title(title) = block(
  width: 100%,
  fill: color-volt,
  inset: (x: px(26), top: px(26), bottom: px(30)),
  outset: (x: margin-x * m-factor, top: margin-top * m-factor),
  below: px(30),
)[
  #text(
    font: font-mono,
    size: tx(11),
    weight: 700,
    tracking: 0.22em,
    fill: rgb(19, 19, 19, 60%),
    "// HIGH VOLT",
  )
  #v(px(16))
  #set text(
    font: hf(1, font-bold),
    size: tx(size-h1),
    weight: 800,
    tracking: 0.02em,
    fill: color-bg,
  )
  #set par(leading: 0.38em, justify: false)
  #title
]

#let signature-row(author: "", date: "", logo: logo-url) = block(
  above: px(34),
  width: 100%,
)[
  #hairline(below: px(16))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(12),
    align: (horizon, left + horizon, right + horizon),
    box(
      width: px(40),
      height: px(40),
      fill: color-panel,
      stroke: px(2) + color-volt,
    )[
      #align(center + horizon, text(
        font: font-bold,
        size: tx(18),
        weight: 800,
        fill: color-peach,
        if author != "" { author.first() } else { "" },
      ))
    ],
    [
      #text(
        font: font-text,
        size: tx(16),
        fill: color-ink,
        weight: 600,
      )[#author] \
      #text(
        font: font-mono,
        size: tx(13),
        fill: color-muted,
        tracking: 0.05em,
      )[#date]
    ],
    if logo != "" { image(logo, width: 50pt, height: 50pt) },
  )
]


#let divider() = block(width: 100%, above: px(34), below: px(34), grid(
  columns: (px(40), 1fr),
  column-gutter: px(10),
  align: horizon,
  line(length: 100%, stroke: px(2) + color-volt),
  line(length: 100%, stroke: px(1) + color-hair),
))

#let conf(doc) = {
  set page(
    ..if paper != "" { (paper: paper, flipped: p-orient == "landscape") } else {
      (
        width: long-page-width,
        height: if paged { long-page-min-height } else { auto },
      )
    },
    margin: (
      left: margin-x * m-factor,
      right: margin-x * m-factor,
      top: margin-top * m-factor,
      bottom: margin-bot * m-factor,
    ),
    fill: gradient.radial(color-bg2, color-bg, center: (18%, 4%), radius: 120%),
    footer: if paged {
      context align(center)[
        #set text(
          font: font-mono,
          size: tx(size-caption),
          fill: color-muted,
          tracking: 0.1em,
        )
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
  set text(
    font: font-text,
    fill: color-ink,
    size: tx(size-body),
    lang: "zh",
    weight: 400,
    tracking: 0.01em,
  )
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.82em * l-factor,
    spacing: px(18),
    first-line-indent: (
      amount: if p-indent == "true" { 2em } else { 0pt },
      all: false,
    ),
  )

  show strong: it => highlight(
    fill: color-chip,
    top-edge: 0.32em,
    bottom-edge: -0.2em,
    extent: px(2),
    text(weight: 800, fill: color-peach, it.body),
  )
  show emph: it => text(style: "italic", fill: color-soft, it.body)

  show heading: set par(justify: false)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%,
    above: px(28),
    below: px(12),
  )[
    #set text(
      font: hf(3, font-bold),
      size: tx(size-h3),
      weight: 700,
      fill: color-peach,
    )
    #set par(leading: 0.40em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(
      font: hf(4, font-text),
      size: tx(size-h4),
      weight: 600,
      fill: color-ink,
    ); #set par(leading: 0.40em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-text),
      size: tx(size-h5),
      weight: 600,
      fill: color-soft,
    ); #set par(leading: 0.40em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-mono),
      size: tx(size-h6),
      weight: 500,
      fill: color-muted,
      tracking: 0.1em,
    ); #set par(leading: 0.40em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, block(clip: true, stroke: px(2) + color-volt, {
          set image(width: w)
          it
        }))
      })
    } else { it }
  }

  show link: it => text(fill: color-peach, weight: 600, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(28),
    below: px(28),
    fill: color-panel,
    stroke: (left: px(4) + color-volt, rest: px(1) + color-line),
    inset: (left: px(22), rest: px(18)),
  )[
    #text(
      font: font-mono,
      size: tx(11),
      weight: 500,
      fill: color-volt,
      tracking: 0.2em,
      "// QUOTE",
    )
    #v(px(8))
    #set text(size: tx(16), fill: color-soft, style: "italic")
    #it.body
  ]

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-peach,
  )
  show raw.where(block: false): box.with(
    fill: color-panel,
    inset: (x: px(5), y: px(1)),
    outset: (y: px(2)),
    stroke: px(1) + color-hair,
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(26),
    below: px(26),
    fill: color-panel2,
    stroke: px(2) + color-volt,
    inset: px(18),
  )[
    #block(below: px(12))[
      #text(
        font: font-mono,
        size: tx(11),
        weight: 500,
        fill: color-volt,
        tracking: 0.2em,
        "// CODE",
      )
    ]
    #set par(leading: 0.5em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: color-peach-hi)
    #it
  ]

  set list(
    marker: text(fill: color-volt, weight: 700)[▸],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(24),
      height: px(24),
      fill: color-panel,
      stroke: px(1) + color-volt,
      align(center + horizon, text(
        font: font-mono,
        size: tx(13),
        weight: 700,
        fill: color-peach,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd,
    inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-panel3 } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: it => block(
    width: 100%,
    clip: true,
    stroke: px(1) + color-hair,
    breakable: true,
    it,
  )
  show table: set text(size: tx(15), fill: color-soft)
  show table.cell.where(y: 0): set text(
    font: font-mono,
    weight: 700,
    fill: color-peach,
    tracking: 0.08em,
  )

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
