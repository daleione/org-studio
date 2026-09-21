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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#1d1b16"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#444653"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#fff9f0"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f9f3ea"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#f3ede4"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#001e73",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#fe5f00"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "38"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "25"))
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

#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Avenir Next"),
  "Hiragino Sans GB",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Avenir Next"),
  "Hiragino Sans GB",
)
#let font-serif = (
  sys.inputs.at("theme-font-heading", default: "Copperplate"),
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
#let color-card = theme-panel
#let color-card2 = theme-border
#let color-navy = theme-primary
#let color-navy-dk = rgb("#001355")
#let color-cobalt = rgb("#002fa7")
#let color-orange = theme-accent
#let color-rust = rgb("#a63b00")
#let color-peach = rgb("#ffdbce")
#let color-magenta = rgb("#c2185b")
#let color-teal = rgb("#004824")
#let color-emerald = rgb("#33c070")
#let color-mint = rgb("#77fca5")
#let color-ink = theme-ink
#let color-soft = theme-muted
#let color-muted = rgb(68, 70, 83, 60%)
#let color-line = rgb("#c4c5d6")
#let color-hair = rgb(0, 30, 115, 15%)
#let color-navytint = rgb(0, 30, 115, 8%)
#let color-orangetint = rgb(254, 95, 0, 14%)

#let hairline(above: px(0), below: px(0)) = block(
  width: 100%,
  above: above,
  below: below,
  line(length: 100%, stroke: px(2) + color-hair),
)

#let cutout(w, h, fill, dx: px(0), dy: px(0), rot: 0deg) = place(
  dx: dx,
  dy: dy,
  rotate(rot, box(width: w, height: h, fill: fill, radius: (
    top-left: px(18),
    top-right: px(7),
    bottom-right: px(20),
    bottom-left: px(6),
  ))),
)

#let h1-block(body) = block(width: 100%, above: px(34), below: px(22))[
  #box(width: 100%, height: px(20))[
    #cutout(px(34), px(16), color-orange, dx: px(0), dy: px(2), rot: -6deg)
    #cutout(px(20), px(16), color-magenta, dx: px(42), dy: px(0), rot: 5deg)
    #cutout(px(16), px(16), color-emerald, dx: px(70), dy: px(3), rot: -3deg)
  ]
  #v(px(8))
  #set text(
    font: hf(1, font-serif),
    size: tx(size-h1),
    weight: 800,
    tracking: 0.02em,
    fill: color-navy,
  )
  #set par(leading: 0.42em)
  #body
  #v(px(16))
  #line(length: 100%, stroke: px(4) + color-navy)
]

#let h2-block(body) = block(width: 100%, above: px(34), below: px(16))[
  #grid(
    columns: (auto, 1fr),
    column-gutter: px(13),
    align: horizon,
    box(width: px(13), height: px(28), fill: color-orange, radius: (
      top-left: px(8),
      top-right: px(2),
      bottom-right: px(9),
      bottom-left: px(3),
    )),
    {
      set text(
        font: hf(2, font-serif),
        size: tx(size-h2),
        weight: 700,
        fill: color-navy,
      )
      set par(leading: 0.42em)
      body
    },
  )
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  let dots = (color-orange, color-magenta, color-emerald, color-cobalt)
  block(width: 100%, above: px(22), below: px(10))[
    #text(
      font: font-text,
      size: tx(13),
      weight: 700,
      fill: color-navy,
      tracking: 0.2em,
    )[CUTOUT CONTENTS · CONTENTS]
  ]
  block(width: 100%, below: px(18))[
    #box(width: 100%)[
      #place(dx: px(8), dy: px(8), block(
        width: 100%,
        height: 100%,
        radius: px(10),
        fill: color-navy,
      ))
      #block(
        width: 100%,
        fill: color-card,
        stroke: px(2) + color-navy,
        radius: px(10),
        inset: px(20),
      )[
        #set par(leading: 0.6em)
        #stack(spacing: px(13), ..hs
          .enumerate()
          .map(((i, h)) => grid(
            columns: (auto, 1fr),
            column-gutter: px(12),
            align: horizon,
            box(
              width: px(15),
              height: px(15),
              fill: dots.at(calc.rem(i, dots.len())),
              radius: (
                top-left: px(7),
                top-right: px(2),
                bottom-right: px(8),
                bottom-left: px(2),
              ),
            ),
            link(h.location(), text(
              fill: color-navy-dk,
              weight: 600,
              size: tx(16),
              h.body,
            )),
          )))
      ]
    ]
  ]
  hairline(above: px(18), below: px(0))
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(
  above: px(34),
  width: 100%,
)[
  #hairline(below: px(16))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(12),
    align: (horizon, left + horizon, right + horizon),
    box(rotate(-4deg, box(
      width: px(42),
      height: px(42),
      fill: color-orange,
      stroke: px(2) + color-navy,
      radius: (
        top-left: px(16),
        top-right: px(5),
        bottom-right: px(18),
        bottom-left: px(5),
      ),
    )[
      #align(center + horizon, text(
        font: font-bold,
        size: tx(18),
        weight: 700,
        fill: color-navy,
        if author != "" { author.first() } else { "" },
      ))
    ])),
    [
      #text(
        font: font-text,
        size: tx(16),
        fill: color-ink,
        weight: 600,
      )[#author] \
      #text(font: font-text, size: tx(14), fill: color-muted)[#date]
    ],
    if logo != "" { image(logo, width: 50pt, height: 50pt) },
  )
]


#let divider() = block(width: 100%, above: px(34), below: px(34), grid(
  columns: (px(40), px(26), px(20), 1fr),
  rows: px(5),
  column-gutter: px(7),
  box(fill: color-orange, width: 100%, height: 100%, radius: (
    top-left: px(4),
    bottom-right: px(5),
  )),
  box(fill: color-magenta, width: 100%, height: 100%, radius: (
    top-left: px(4),
    bottom-right: px(5),
  )),
  box(fill: color-emerald, width: 100%, height: 100%, radius: (
    top-left: px(4),
    bottom-right: px(5),
  )),
  box(fill: color-hair, width: 100%, height: 100%, radius: px(2)),
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
    fill: color-orangetint,
    top-edge: 0.32em,
    bottom-edge: -0.2em,
    extent: px(2),
    radius: (
      top-left: px(6),
      top-right: px(2),
      bottom-right: px(7),
      bottom-left: px(2),
    ),
    text(weight: 700, fill: color-rust, it.body),
  )
  show emph: it => text(style: "italic", fill: color-orange, it.body)

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
      fill: color-cobalt,
    )
    #set par(leading: 0.42em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(
      font: hf(4, font-text),
      size: tx(size-h4),
      weight: 600,
      fill: color-navy,
    ); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-text),
      size: tx(size-h5),
      weight: 600,
      fill: color-soft,
    ); #set par(leading: 0.42em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-text),
      size: tx(size-h6),
      weight: 600,
      fill: color-muted,
    ); #set par(leading: 0.42em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, box(width: w)[
          #place(dx: px(8), dy: px(8), block(
            width: 100%,
            height: 100%,
            radius: px(6),
            fill: color-navy,
          ))
          #block(radius: px(6), clip: true, stroke: px(2) + color-navy, {
            set image(width: w)
            it
          })
        ])
      })
    } else { it }
  }

  show link: it => text(fill: color-cobalt, weight: 600, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(26),
    below: px(26),
    fill: color-peach,
    stroke: (left: px(8) + color-orange, rest: none),
    radius: (right: px(20)),
    inset: (left: px(22), rest: px(20)),
  )[
    #text(font: font-serif, size: tx(30), weight: 700, fill: color-orange)[“]
    #v(px(2))
    #set text(
      font: font-serif,
      size: tx(17),
      fill: color-navy-dk,
      style: "italic",
      weight: 500,
    )
    #it.body
  ]

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-rust,
  )
  show raw.where(block: false): box.with(
    fill: color-card2,
    inset: (x: px(5), y: px(1)),
    radius: px(4),
    outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-emerald,
    radius: px(16),
    clip: true,
    inset: (left: px(8), rest: px(0)),
  )[
    #block(
      width: 100%,
      fill: color-ink,
      radius: (left: px(8), right: px(16)),
      inset: px(20),
    )[
      #set par(leading: 0.5em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: color-mint)
      #it
    ]
  ]

  set list(
    marker: text(fill: color-orange, weight: 700)[●],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(22),
      height: px(22),
      fill: color-navy,
      radius: (
        top-left: px(10),
        top-right: px(3),
        bottom-right: px(11),
        bottom-left: px(3),
      ),
      align(center + horizon, text(
        size: tx(13),
        weight: 700,
        fill: color-bg,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(1) + color-navy,
    inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-navy } else if calc.even(row) {
      color-card
    } else { none },
  )
  show table: it => block(
    width: 100%,
    radius: px(8),
    clip: true,
    stroke: px(2) + color-navy,
    breakable: true,
    it,
  )
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
