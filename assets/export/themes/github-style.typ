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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#1f2328"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#57606a"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#ffffff"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f6f8fa"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#d8dee4"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#0969da",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#0550ae"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "28"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "23"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "21"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "18"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "16"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "15"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "16"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "13"))
#let long-page-width = 540pt
#let margin-x = 39pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt
#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Helvetica Neue"),
  "PingFang SC",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Helvetica Neue"),
  "PingFang SC",
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
#let color-text = theme-ink
#let color-muted = theme-muted
#let color-accent = theme-primary
#let color-accent2 = rgb("#218bff")
#let color-strong = theme-accent
#let color-border = theme-border
#let color-panel = theme-panel
#let color-table-bd = rgb("#d1d9e0")
#let color-zebra = rgb("#f9f9f9")

#let h1-block(body) = block(
  width: 100%,
  above: px(36),
  below: px(18),
  fill: gradient.linear(rgb("#f8fbff"), rgb("#f6f8fa"), angle: 90deg),
  stroke: (left: px(5) + color-accent, rest: px(1) + color-border),
  radius: (right: px(10)),
  inset: (left: px(16), right: px(16), top: px(12), bottom: px(11)),
)[
  #set text(
    font: hf(1, font-bold),
    size: tx(size-h1),
    weight: 600,
    tracking: 0.01em,
    fill: color-text,
  )
  #set par(leading: 0.4em)
  #body
]

#let h2-block(body) = block(width: 100%, above: px(34), below: px(18))[
  #set par(leading: 0.45em)
  #stack(
    {
      set text(
        font: hf(2, font-bold),
        size: tx(size-h2),
        weight: 700,
        fill: color-text,
      )
      body
    },
    v(px(10)),
    line(length: 100%, stroke: px(2) + color-accent),
    line(length: 100%, stroke: px(2) + color-border),
  )
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
      text(fill: color-accent, weight: 700, size: tx(15))[•],
      link(h.location(), text(fill: color-accent, size: tx(15), h.body)),
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
        font: font-text,
        size: tx(15),
        fill: color-text,
        weight: 600,
      )[#author] \
      #text(font: font-text, size: tx(15), fill: color-muted)[#date]
    ],
    if logo != "" { image(logo, width: 56pt, height: 56pt) },
  )
]


#let divider() = align(center, block(
  width: 100%,
  height: px(1),
  above: px(34),
  below: px(34),
  fill: gradient.linear(
    (rgb("#d1d9e000"), 0%),
    (color-table-bd, 16%),
    (color-accent, 50%),
    (color-table-bd, 84%),
    (rgb("#d1d9e000"), 100%),
    dir: ltr,
  ),
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
    fill: color-text,
    size: tx(size-body),
    lang: "zh",
    weight: 400,
  )
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.6em * l-factor,
    spacing: px(18),
    first-line-indent: (
      amount: if p-indent == "true" { 2em } else { 0pt },
      all: false,
    ),
  )

  show strong: it => highlight(
    fill: rgb(9, 105, 218, 14%),
    top-edge: 0.32em,
    bottom-edge: -0.18em,
    extent: px(2),
    text(weight: 600, fill: color-strong, it.body),
  )
  show emph: it => text(style: "italic", fill: color-text, it.body)

  show heading: set par(justify: false)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%,
    above: px(28),
    below: px(14),
    stroke: (left: px(3) + color-accent2),
    inset: (left: px(12), y: 2pt),
  )[
    #set text(
      font: hf(3, font-bold),
      size: tx(size-h3),
      weight: 600,
      fill: color-text,
    )
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(
      font: hf(4, font-text),
      size: tx(size-h4),
      weight: 600,
      fill: color-text,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-text),
      size: tx(size-h5),
      weight: 600,
      fill: color-muted,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-text),
      size: tx(size-h6),
      weight: 600,
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

  show link: it => text(fill: color-accent, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(18),
    below: px(18),
    fill: color-panel,
    stroke: (left: px(4) + color-accent2, rest: px(1) + color-border),
    radius: (right: px(10)),
    inset: (left: px(16), right: px(16), y: px(14)),
    {
      set text(size: tx(16), fill: color-text)
      it.body
    },
  )

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-text,
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(16),
    below: px(16),
    fill: color-panel,
    radius: px(6),
    clip: true,
  )[
    #block(width: 100%, height: px(36), fill: rgb("#eef1f4"), inset: (
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
    #block(width: 100%, inset: px(16))[
      #set par(leading: 0.45em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: color-text)
      #it
    ]
  ]

  set list(
    marker: text(fill: color-accent, weight: 700)[•],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(22),
      height: px(22),
      radius: px(11),
      fill: rgb(9, 105, 218, 10%),
      align(center + horizon, text(
        size: tx(13),
        weight: 700,
        fill: color-accent,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd,
    inset: (x: px(16), y: px(8)),
    fill: (_, row) => if row == 0 { color-panel } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: it => block(
    width: 100%,
    radius: px(8),
    clip: true,
    stroke: px(1) + color-table-bd,
    breakable: true,
    it,
  )
  show table: set text(size: tx(15))

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
