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
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#8a7d68"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#fff3e1"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#fdebd2"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e9d4b3"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#fa2d1a",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#2e4c8c"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "44"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "23"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "21"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "17"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "15"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "14"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "15"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "14"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let long-page-width = float(sys.inputs.at("long-page-width-pt", default: "540")) * 1pt
#let margin-x = float(sys.inputs.at("long-page-margin-x-pt", default: "39")) * 1pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt
#let content-width = long-page-width - margin-x * 2
#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Avenir Next"),
  "Hiragino Sans GB",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Impact"),
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
#let color-text = theme-ink
#let color-red = theme-primary
#let color-blue = theme-accent
#let color-em = rgb("#6b5d4a")
#let color-muted = theme-muted
#let color-panel = theme-panel
#let color-zebra = rgb("#fbe7cf")
#let color-tablebd = theme-border

#let tilt(angle, body) = box(rotate(angle, reflow: true, body))

#let kicker(label, fill: color-red, angle: -3deg) = block(
  above: px(4),
  below: px(14),
)[
  #tilt(angle, box(
    fill: fill,
    inset: (x: px(14), y: px(8)),
  )[
    #set text(
      font: font-bold,
      size: tx(13),
      weight: 900,
      fill: white,
      tracking: 0.22em,
    )
    #label
  ])
]

#let h1-block(body) = block(width: 100%, above: px(34), below: px(22), layout(
  size => {
    let w = size.width
    let inner = block(
      width: w - px(8),
      fill: color-ink,
      inset: (x: px(22), y: px(20)),
    )[
      #set text(
        font: hf(1, font-bold),
        size: tx(size-h1),
        weight: 900,
        fill: color-bg,
        tracking: 0em,
      )
      #set par(leading: 0.42em, justify: false)
      #body
    ]
    context {
      let s = measure(inner)
      block(width: s.width + px(8), height: s.height + px(8))[
        #place(top + left, dx: px(8), dy: px(8), rect(
          width: s.width,
          height: s.height,
          fill: color-red,
        ))
        #place(top + left, inner)
      ]
    }
  },
))

#let h2-counter = counter("zsp-h2")
#let h2-block(body) = {
  h2-counter.step()
  context {
    let i = h2-counter.get().first()
    let fill = if calc.even(i) { color-blue } else { color-red }
    let angle = if calc.even(i) { 1.5deg } else { -1.5deg }
    block(width: 100%, above: px(36), below: px(20))[
      #tilt(angle, block(
        width: content-width,
        fill: fill,
        inset: (left: px(18), right: px(16), top: px(13), bottom: px(12)),
      )[
        #set text(
          font: hf(2, font-bold),
          size: tx(size-h2),
          weight: 900,
          fill: white,
          tracking: 0.04em,
        )
        #set par(leading: 0.42em)
        #grid(
          columns: (auto, 1fr),
          column-gutter: px(12),
          align: horizon,
          box(rect(width: px(10), height: px(26), fill: color-bg)), body,
        )
      ])
    ]
  }
}

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  kicker("CONTENTS　CONTENTS", fill: color-ink, angle: -3deg)
  let toc-item(i, h) = {
    let c = if calc.even(i) { color-blue } else { color-red }
    let inner = box(
      width: content-width - px(8),
      fill: color-panel,
      stroke: px(2) + c,
      inset: (left: px(10), right: px(14), y: px(8)),
      grid(
        columns: (auto, 1fr),
        column-gutter: px(10),
        align: horizon,
        box(fill: c, inset: (x: px(7), y: px(3)), text(
          font: font-bold,
          size: tx(13),
          weight: 900,
          fill: white,
          str(i + 1),
        )),
        link(h.location(), text(
          fill: color-ink,
          weight: 700,
          size: tx(16),
          h.body,
        )),
      ),
    )
    tilt(if calc.even(i) { -1deg } else { 1deg }, inner)
  }
  block(width: 100%, below: px(18))[
    #set par(leading: 0.6em)
    #stack(spacing: px(10), ..hs.enumerate().map(((i, h)) => toc-item(i, h)))
  ]
}

#let poster-title(title) = block[
  #kicker("LEARNING NOTES", fill: color-blue, angle: -3deg)
  #h1-block(title)
]

#let signature-row(author: "", date: "", logo: logo-url) = block(
  above: px(36),
  width: 100%,
)[
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(14),
    align: (horizon, left + horizon, right + horizon),
    tilt(-3deg, box(fill: color-ink, inset: px(12))[
      #align(center + horizon, text(
        font: font-bold,
        size: tx(20),
        weight: 900,
        fill: color-bg,
        if author != "" { author.first() } else { "" },
      ))
    ]),
    [
      #text(
        font: font-text,
        size: tx(16),
        fill: color-ink,
        weight: 700,
      )[#author] \
      #text(font: font-text, size: tx(14), fill: color-muted)[#date]
    ],
    if logo != "" { image(logo, width: 52pt, height: 52pt) },
  )
]


#let divider() = block(width: 100%, above: px(36), below: px(36), grid(
  columns: (1fr, 1fr),
  rows: px(5),
  box(fill: color-red, width: 100%, height: 100%),
  box(fill: color-blue, width: 100%, height: 100%),
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
    tracking: 0.02em,
  )
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.78em * l-factor,
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
    text(font: font-bold, weight: 900, fill: white, it.body),
  )
  show emph: it => text(style: "italic", fill: color-em, it.body)

  show heading: set par(justify: false)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%,
    above: px(28),
    below: px(14),
    stroke: (left: px(5) + color-blue),
    inset: (left: px(12), y: 2pt),
  )[
    #set text(
      font: hf(3, font-bold),
      size: tx(size-h3),
      weight: 900,
      fill: color-ink,
    )
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(
      font: hf(4, font-bold),
      size: tx(size-h4),
      weight: 700,
      fill: color-red,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-text),
      size: tx(size-h5),
      weight: 700,
      fill: color-blue,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-text),
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

  show link: it => text(fill: color-red, weight: 700, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(28),
    below: px(28),
  )[
    #block(width: 100%, fill: color-ink, inset: (x: px(22), y: px(20)))[
      #text(
        font: font-bold,
        size: tx(13),
        weight: 900,
        fill: color-red,
        tracking: 0.2em,
      )[QUOTE] \
      #v(px(10))
      #set text(size: tx(17), fill: color-bg, weight: 600)
      #set par(leading: 0.6em)
      #it.body
    ]
  ]

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-red,
  )
  show raw.where(block: false): box.with(
    fill: color-panel,
    stroke: px(1.5) + color-red,
    inset: (x: px(5), y: px(1)),
    outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-panel,
    stroke: px(2.5) + color-ink,
    clip: true,
  )[
    #block(width: 100%, height: px(8), fill: gradient.linear(
      (color-red, 0%),
      (color-red, 50%),
      (color-blue, 50%),
      (color-blue, 100%),
      dir: ltr,
    ))
    #block(width: 100%, inset: px(18))[
      #set par(leading: 0.45em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: color-ink)
      #it
    ]
  ]

  set list(
    marker: box(fill: color-red, inset: px(3), outset: (y: px(1))),
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(22),
      height: px(22),
      fill: if calc.odd(n) { color-red } else { color-blue },
      align(center + horizon, text(
        font: font-bold,
        size: tx(13),
        weight: 900,
        fill: white,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(2) + color-ink,
    inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-blue } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: it => block(
    width: 100%,
    clip: true,
    stroke: px(2.5) + color-ink,
    breakable: true,
    it,
  )
  show table: set text(size: tx(15), fill: color-ink)
  show table.cell.where(y: 0): set text(
    weight: 900,
    fill: white,
    font: font-bold,
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
