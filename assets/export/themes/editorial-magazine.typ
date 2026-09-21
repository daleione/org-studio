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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#1a1a1a"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#8c8c8c"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#fafaf8"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#ffffff"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e6e6e2"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#ff2442",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#ff2442"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "36"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "20"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "21"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "17"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "15"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "14"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "15"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "14"))
#let long-page-width = 540pt
#let margin-x = 39pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt
#let content-width = long-page-width - margin-x * 2
#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Baskerville"),
  "Songti SC",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Didot"),
  "Songti SC",
)
#let font-label = ("Helvetica Neue", "PingFang SC")
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
#let color-text = theme-ink
#let color-soft = rgb("#5c5c5c")
#let color-muted = theme-muted
#let color-red = theme-primary
#let color-red-bg = rgb(255, 36, 66, 8%)
#let color-card = theme-panel
#let color-border = rgb(0, 0, 0, 8%)
#let color-line = rgb(0, 0, 0, 8%)
#let color-table-bd = theme-border
#let color-zebra = rgb("#f4f4f1")

#let h1-block(body) = block(
  width: 100%,
  above: px(34),
  below: px(22),
  fill: color-card,
  stroke: px(1) + color-border,
  radius: px(14),
  inset: (x: px(22), top: px(20), bottom: px(22)),
)[
  #set text(
    font: hf(1, font-bold),
    size: tx(size-h1),
    weight: 900,
    tracking: -0.02em,
    fill: color-text,
  )
  #set par(leading: 0.42em)
  #body
  #v(px(14))
  #box(width: px(48), height: px(4), radius: px(2), fill: color-red)
]

#let h2-block(body) = block(
  width: 100%,
  above: px(34),
  below: px(18),
)[
  #box(
    fill: color-red-bg,
    radius: px(20),
    inset: (x: px(16), y: px(9)),
  )[
    #set text(
      font: hf(2, font-bold),
      size: tx(size-h2),
      weight: 600,
      fill: color-red,
    )
    #set par(leading: 0.4em)
    #body
  ]
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  h2-block[CONTENTS]
  block(
    width: 100%,
    below: px(16),
    fill: color-card,
    stroke: px(1) + color-border,
    radius: px(12),
    inset: (x: px(18), y: px(16)),
  )[
    #set par(leading: 0.6em)
    #stack(spacing: px(11), ..hs
      .enumerate()
      .map(((i, h)) => grid(
        columns: (auto, 1fr),
        column-gutter: px(12),
        align: horizon,
        text(
          font: font-label,
          fill: color-red,
          weight: 600,
          size: tx(14),
          "0" + str(i + 1),
        ),
        link(h.location(), text(
          font: font-text,
          fill: color-text,
          weight: 500,
          size: tx(16),
          h.body,
        )),
      )))
  ]
}

// Magazine masthead: the band bleeds to the page edges, so the first screen has
// a silhouette instead of another rounded card.
#let poster-title(title) = {
  block(
    width: 100%,
    fill: color-red,
    inset: (x: px(26), top: px(30), bottom: px(34)),
    outset: (x: margin-x * m-factor, top: margin-top * m-factor),
  )[
    #set text(
      font: hf(1, font-bold),
      size: tx(size-h1),
      weight: 900,
      tracking: -0.02em,
      fill: white,
    )
    #set par(leading: 0.38em)
    #title
  ]
  block(width: 100%, above: px(16), below: px(26))[
    #text(
      font: font-text,
      size: tx(11),
      weight: 700,
      tracking: 0.26em,
      fill: color-red,
    )[EDITORIAL MAGAZINE]
  ]
}

#let signature-row(author: "", date: "", logo: logo-url) = block(
  above: px(36),
  width: 100%,
)[
  #line(length: 100%, stroke: px(1) + color-line)
  #v(px(14))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(14),
    align: (horizon, left + horizon, right + horizon),
    box(circle(
      radius: px(24),
      fill: rgb(48, 48, 52, 8%),
      stroke: px(1) + color-border,
    )[
      #align(center + horizon, text(
        font: font-bold,
        size: tx(20),
        weight: 600,
        fill: color-soft,
        if author != "" { author.first() } else { "" },
      ))
    ]),
    [
      #text(
        font: font-bold,
        size: tx(17),
        fill: color-text,
        weight: 700,
      )[#author] \
      #text(
        font: font-label,
        size: tx(13),
        fill: color-muted,
        tracking: 0.1em,
      )[#date]
    ],
    if logo != "" { image(logo, width: 52pt, height: 52pt) },
  )
]


#let divider() = align(center, block(above: px(38), below: px(38))[
  #grid(
    columns: (1fr, auto, 1fr),
    column-gutter: px(14),
    align: horizon,
    line(length: 100%, stroke: px(1) + color-line),
    box(circle(radius: px(3), fill: color-red)),
    line(length: 100%, stroke: px(1) + color-line),
  )
])

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
  set text(
    font: font-text,
    fill: color-text,
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

  show strong: it => text(
    font: font-bold,
    weight: 700,
    fill: color-red,
    it.body,
  )
  show emph: it => text(style: "italic", fill: color-soft, it.body)

  show heading: set par(justify: false)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%,
    above: px(28),
    below: px(14),
    stroke: (left: px(4) + color-red),
    inset: (left: px(12), y: 2pt),
  )[
    #set text(
      font: hf(3, font-bold),
      size: tx(size-h3),
      weight: 700,
      fill: color-text,
    )
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(
      font: hf(4, font-bold),
      size: tx(size-h4),
      weight: 700,
      fill: color-text,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-bold),
      size: tx(size-h5),
      weight: 700,
      fill: color-soft,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-bold),
      size: tx(size-h6),
      weight: 700,
      fill: color-muted,
    ); #set par(leading: 0.4em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, {
          set image(width: w)
          it
        })
      })
    } else { it }
  }

  show link: it => text(fill: color-red, weight: 500, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(28),
    below: px(28),
    fill: color-card,
    stroke: px(1) + color-border,
    radius: px(14),
    inset: (left: px(22), right: px(20), top: px(30), bottom: px(20)),
  )[
    #place(top + left, dx: px(10), dy: px(-18), text(
      font: ("Songti SC",),
      size: tx(58),
      weight: 700,
      fill: color-red-bg,
    )["])
    #set text(font: font-text, size: tx(17), fill: color-text, weight: 500)
    #set par(leading: 0.7em)
    #it.body
  ]

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-red,
  )
  show raw.where(block: false): box.with(
    fill: color-red-bg,
    inset: (x: px(5), y: px(1)),
    radius: px(4),
    outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-card,
    stroke: px(1) + color-border,
    radius: px(10),
    clip: true,
  )[
    #block(width: 100%, height: px(8), fill: color-red)
    #block(width: 100%, inset: px(18))[
      #set par(leading: 0.45em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: color-soft)
      #it
    ]
  ]

  set list(
    marker: text(fill: color-red, weight: 700)[•],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(22),
      height: px(22),
      radius: px(11),
      fill: color-red,
      align(center + horizon, text(
        font: font-label,
        size: tx(13),
        weight: 700,
        fill: white,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd,
    inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-red-bg } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: it => block(
    width: 100%,
    radius: px(10),
    clip: true,
    stroke: px(1) + color-table-bd,
    breakable: true,
    it,
  )
  show table: set text(size: tx(15))
  show table.cell.where(y: 0): set text(
    font: font-bold,
    weight: 700,
    fill: color-red,
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
