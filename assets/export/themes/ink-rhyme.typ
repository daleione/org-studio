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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#111111"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#555555"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#ffffff"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f5f5f5"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#cccccc"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#000000",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#222222"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "32"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "22"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "20"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "17"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "15"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "14"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "16"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "13"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let long-page-width = 540pt
#let margin-x = 39pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt
#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Songti SC"),
  "Kaiti SC",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Kaiti SC"),
  "Songti SC",
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
#let color-ink = theme-primary
#let color-text = theme-ink
#let color-h3 = theme-accent
#let color-em = theme-muted
#let color-panel = theme-panel
#let color-border = rgb("#e0e0e0")
#let color-table-bd = theme-border
#let color-zebra = rgb("#f9f9f9")

#let h1-block(body) = block(
  width: 100%,
  above: px(36),
  below: px(18),
  stroke: (bottom: px(2) + color-ink),
  inset: (bottom: px(10)),
)[
  #set text(
    font: hf(1, font-bold),
    size: tx(size-h1),
    weight: 700,
    tracking: 0.10em,
    fill: color-ink,
  )
  #set par(leading: 0.4em)
  #body
]

#let h2-block(body) = block(
  width: 100%,
  above: px(34),
  below: px(18),
  stroke: (left: px(3) + color-ink),
  inset: (left: px(12), y: 2pt),
)[
  #set text(
    font: hf(2, font-bold),
    size: tx(size-h2),
    weight: 700,
    fill: color-text,
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
      text(fill: color-ink, weight: 700, size: tx(15))[•],
      link(h.location(), underline(stroke: 0.5pt + color-ink, offset: 2pt, text(
        fill: color-ink,
        size: tx(15),
        h.body,
      ))),
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
      #text(font: font-text, size: tx(15), fill: color-em)[#date]
    ],
    if logo != "" { image(logo, width: 56pt, height: 56pt) },
  )
]


#let divider() = align(center, block(
  width: 34%,
  height: px(1),
  above: px(38),
  below: px(38),
  fill: gradient.linear(
    (rgb(0, 0, 0, 0%), 0%),
    (rgb(0, 0, 0, 88%), 28%),
    (rgb(0, 0, 0, 88%), 72%),
    (rgb(0, 0, 0, 0%), 100%),
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
    fill: color-text,
    size: tx(size-body),
    lang: "zh",
    weight: 400,
  )
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.8em * l-factor,
    spacing: px(18),
    first-line-indent: (
      amount: if p-indent == "true" { 2em } else { 0pt },
      all: false,
    ),
  )

  show strong: it => text(weight: 600, fill: color-ink, it.body)
  show emph: it => text(style: "italic", fill: color-em, it.body)

  show heading: set par(justify: false)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    above: px(28),
    below: px(14),
    inset: (bottom: px(6)),
  )[
    #set text(
      font: hf(3, font-bold),
      size: tx(size-h3),
      weight: 600,
      fill: color-h3,
    )
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(
      font: hf(4, font-text),
      size: tx(size-h4),
      weight: 600,
      fill: color-h3,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-text),
      size: tx(size-h5),
      weight: 600,
      fill: color-em,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-text),
      size: tx(size-h6),
      weight: 600,
      fill: rgb("#888888"),
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

  show link: it => underline(stroke: 0.5pt + color-ink, offset: 2pt, text(
    fill: color-ink,
    it.body,
  ))

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: rgb("#fafafa"),
    stroke: (left: px(3) + color-ink),
    inset: (left: px(18), right: px(18), y: px(16)),
    {
      set text(size: tx(16), fill: color-text)
      it.body
    },
  )

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-ink,
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-panel,
    radius: px(2),
    clip: true,
    stroke: px(1) + color-border,
  )[
    #block(width: 100%, height: px(36), fill: rgb("#e8e8e8"), inset: (
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
    marker: text(fill: color-ink, weight: 700)[•],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(22),
      height: px(22),
      radius: px(11),
      fill: rgb("#f0f0f0"),
      stroke: px(1) + color-table-bd,
      align(center + horizon, text(
        size: tx(13),
        weight: 700,
        fill: color-ink,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd,
    inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-panel } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: it => block(width: 100%, breakable: true, it)
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
