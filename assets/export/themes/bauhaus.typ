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
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#555555"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#ffffff"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f0f0f0"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e0e0e0"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#e30613",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#004d9f"))

#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "40"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "22"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "20"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "17"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "15"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "14"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "16"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "13"))

#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let long-page-width = float(sys.inputs.at("long-page-width-pt", default: "540")) * 1pt
#let margin-x = float(sys.inputs.at("long-page-margin-x-pt", default: "39")) * 1pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt
#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Futura"),
  "Hiragino Sans GB",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Futura"),
  "Hiragino Sans GB",
)
#let font-heavy = (
  sys.inputs.at("theme-font-heading", default: "Futura"),
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
#let color-ink = theme-ink
#let color-red = theme-primary
#let color-blue = theme-accent
#let color-yellow = rgb("#F5A623")
#let color-em = theme-muted
#let color-panel = theme-panel
#let color-zebra = rgb("#f9f9f9")

#let h1-block(body) = block(width: 100%, above: px(36), below: px(18), layout(
  size => {
    let card = block(
      width: size.width - px(6),
      fill: gradient.linear(
        (color-red, 0%),
        (color-red, 64%),
        (color-blue, 64%),
        (color-blue, 86%),
        (color-yellow, 86%),
        (color-yellow, 100%),
        dir: ltr,
      ),
      stroke: px(3) + color-ink,
      inset: (left: px(14), right: px(14), top: px(10), bottom: px(9)),
    )[
      #set text(
        font: hf(1, font-heavy),
        size: tx(size-h1),
        weight: 900,
        tracking: 0.01em,
        fill: white,
      )
      #set par(leading: 0.3em, justify: false)
      #body
    ]
    let h = measure(card).height
    place(dx: px(6), dy: px(6), rect(
      width: size.width - px(6),
      height: h,
      fill: color-ink,
    ))
    card
  },
))

#let h2-block(body) = block(
  width: 100%,
  above: px(34),
  below: px(18),
  fill: gradient.linear(
    (color-yellow, 0%),
    (color-yellow, 68%),
    (white, 68%),
    (white, 100%),
    dir: ltr,
  ),
  stroke: (
    left: px(10) + color-blue,
    top: px(3) + color-ink,
    bottom: px(3) + color-ink,
  ),
  inset: (x: px(16), y: px(8)),
)[
  #set text(
    font: hf(2, font-heavy),
    size: tx(size-h2),
    weight: 800,
    fill: color-ink,
    tracking: 0.01em,
  )
  #set par(leading: 0.45em)
  #body
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  h2-block[CONTENTS]
  block(width: 100%, below: px(16))[
    #set par(leading: 0.6em)
    #stack(spacing: px(8), ..hs.map(h => grid(
      columns: (auto, 1fr),
      column-gutter: px(8),
      text(fill: color-red, weight: 800, size: tx(15))[•],
      link(h.location(), text(
        fill: color-blue,
        size: tx(15),
        weight: 600,
        h.body,
      )),
    )))
  ]
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(
  above: px(34),
  width: 100%,
)[
  #grid(
    columns: (1fr, auto),
    align: (left + bottom, right + bottom),
    [
      #text(
        font: font-bold,
        size: tx(15),
        fill: color-ink,
        weight: 700,
      )[#author] \
      #text(font: font-text, size: tx(15), fill: color-em)[#date]
    ],
    if logo != "" { image(logo, width: 56pt, height: 56pt) },
  )
]


#let divider() = block(
  width: 100%,
  height: px(6),
  above: px(36),
  below: px(36),
  fill: gradient.linear(
    (color-red, 0%),
    (color-red, 33.33%),
    (color-blue, 33.33%),
    (color-blue, 66.66%),
    (color-yellow, 66.66%),
    (color-yellow, 100%),
    dir: ltr,
  ),
)

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
        #set text(size: tx(size-caption), fill: color-em, tracking: 0pt)
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
  )
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.65em * l-factor,
    spacing: px(18),
    first-line-indent: (
      amount: if p-indent == "true" { 2em } else { 0pt },
      all: false,
    ),
  )

  show strong: it => highlight(
    fill: color-red,
    top-edge: 0.32em,
    bottom-edge: -0.2em,
    extent: px(2),
    text(weight: 700, fill: white, it.body),
  )
  show emph: it => text(style: "italic", fill: color-em, it.body)

  show heading: set par(justify: false)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%,
    above: px(28),
    below: px(14),
    fill: white,
    stroke: (left: px(3) + color-red),
    inset: (left: px(12), right: px(12), y: px(6)),
  )[
    #set text(
      font: hf(3, font-bold),
      size: tx(size-h3),
      weight: 700,
      fill: color-ink,
      tracking: 0.05em,
    )
    #set par(leading: 0.45em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(
      font: hf(4, font-text),
      size: tx(size-h4),
      weight: 700,
      fill: color-blue,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-text),
      size: tx(size-h5),
      weight: 700,
      fill: color-em,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-text),
      size: tx(size-h6),
      weight: 700,
      fill: color-em,
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

  show link: it => text(fill: color-blue, weight: 600, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: gradient.linear(rgb("#fff4d6"), rgb("#fff8ea"), angle: 135deg),
    stroke: (
      left: px(6) + color-yellow,
      right: px(2) + color-blue,
      top: px(2) + color-ink,
    ),
    inset: (left: px(20), right: px(20), y: px(16)),
    {
      set text(size: tx(16), fill: color-ink)
      it.body
    },
  )

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-blue,
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-panel,
    clip: true,
    stroke: (left: px(6) + color-red),
  )[
    #block(width: 100%, height: px(36), fill: rgb("#e0e0e0"), inset: (
      x: px(12),
    ))[
      #align(horizon, stack(
        dir: ltr,
        spacing: px(8),
        circle(radius: px(6), fill: rgb("#FF5F56")),
        circle(radius: px(6), fill: rgb("#FFBD2E")),
        circle(radius: px(6), fill: rgb("#27C93F")),
      ))
    ]
    #block(width: 100%, inset: px(20))[
      #set par(leading: 0.45em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: color-ink)
      #it
    ]
  ]

  set list(
    marker: text(fill: color-red, weight: 800)[•],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(24),
      height: px(24),
      fill: color-blue,
      align(center + horizon, text(
        size: tx(13),
        weight: 800,
        fill: white,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(3) + color-ink,
    inset: px(16),
    fill: (_, row) => if row == 0 { color-blue } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: set text(size: tx(15), weight: 500)
  show table.cell.where(y: 0): set text(fill: white, weight: 700)

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
