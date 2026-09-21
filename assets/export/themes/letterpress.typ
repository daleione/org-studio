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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#191c1d"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#445d80"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#ffffff"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#f8f9fa"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#d9dee6"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#001f3f",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#a04100"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "30"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "24"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "20"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "17"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "15"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "13"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "15"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "14"))
#let long-page-width = float(sys.inputs.at("long-page-width-pt", default: "540")) * 1pt
#let margin-x = float(sys.inputs.at("long-page-margin-x-pt", default: "39")) * 1pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt

#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Georgia"),
  "Songti SC",
)
#let font-serif = (
  sys.inputs.at("theme-font-heading", default: "American Typewriter"),
  "Songti SC",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "American Typewriter"),
  "Songti SC",
)
#let font-mono = (
  sys.inputs.at("theme-font-mono", default: "Courier New"),
  "DejaVu Sans Mono",
  "PingFang SC",
  "Apple Color Emoji",
)
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg = theme-bg
#let color-navy = theme-primary
#let color-navy2 = rgb("#0b2a4a")
#let color-rust = theme-accent
#let color-rust-hi = rgb("#ff6b00")
#let color-ink = theme-ink
#let color-soft = theme-muted
#let color-muted = rgb(0, 31, 63, 55%)
#let color-faint = rgb(0, 31, 63, 22%)
#let color-paper = theme-panel
#let color-chip = rgb(160, 65, 0, 9%)
#let color-hair = rgb(0, 31, 63, 14%)
#let color-line = rgb(0, 31, 63, 9%)
#let color-zebra = rgb("#fbfbfa")
#let color-table-bd = rgb(0, 31, 63, 18%)

#let hairline(above: px(0), below: px(0)) = block(
  width: 100%,
  above: above,
  below: below,
  line(length: 100%, stroke: (
    paint: color-hair,
    thickness: px(1),
    dash: "dashed",
  )),
)

#let h1-block(body) = block(width: 100%, above: px(34), below: px(22))[
  #box(fill: color-rust, inset: (x: px(8), y: px(3)), radius: px(2), text(
    font: font-mono,
    size: tx(11),
    weight: 700,
    fill: white,
    tracking: 0.18em,
    "PUBLISHED",
  ))
  #v(px(14))
  #grid(
    columns: (auto, 1fr),
    column-gutter: px(14),
    align: top,
    box(width: px(6), height: px(40), fill: color-rust-hi),
    {
      set text(
        font: hf(1, font-serif),
        size: tx(size-h1),
        weight: 700,
        fill: color-navy,
        tracking: 0.04em,
      )
      set par(leading: 0.4em, justify: false)
      body
    },
  )
  #v(px(16))
  #line(length: 100%, stroke: px(2) + color-navy)
]

#let h2-block(body) = block(width: 100%, above: px(34), below: px(16))[
  #grid(
    columns: (auto, 1fr),
    column-gutter: px(12),
    align: horizon,
    box(width: px(8), height: px(26), fill: color-rust),
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
  #v(px(8))
  #line(length: 100%, stroke: px(1) + color-hair)
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(20), below: px(8))[
    #text(
      font: font-mono,
      size: tx(12),
      weight: 700,
      fill: color-rust,
      tracking: 0.2em,
      "CONTENTS · CONTENTS",
    )
  ]
  block(
    width: 100%,
    below: px(16),
    fill: color-paper,
    stroke: px(2) + color-navy,
    inset: px(18),
  )[
    #set par(leading: 0.6em)
    #stack(spacing: px(12), ..hs
      .enumerate()
      .map(((i, h)) => grid(
        columns: (auto, 1fr),
        column-gutter: px(12),
        align: horizon,
        text(
          font: font-mono,
          size: tx(13),
          weight: 700,
          fill: color-rust,
          "0" + str(i + 1),
        ),
        link(h.location(), text(
          font: font-text,
          fill: color-navy,
          weight: 500,
          size: tx(16),
          h.body,
        )),
      )))
  ]
  hairline(above: px(18), below: px(0))
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(
  above: px(34),
  width: 100%,
)[
  #block(width: 100%, fill: color-navy, inset: px(20))[
    #grid(
      columns: (auto, 1fr, auto),
      column-gutter: px(12),
      align: (horizon, left + horizon, right + horizon),
      box(fill: color-rust, width: px(40), height: px(40))[
        #align(center + horizon, text(
          font: font-serif,
          size: tx(20),
          weight: 700,
          fill: white,
          if author != "" { author.first() } else { "" },
        ))
      ],
      [
        #text(
          font: font-text,
          size: tx(16),
          fill: white,
          weight: 600,
        )[#author] \
        #text(
          font: font-mono,
          size: tx(12),
          fill: rgb(248, 249, 250, 60%),
          tracking: 0.1em,
        )[#date]
      ],
      if logo != "" { image(logo, width: 50pt, height: 50pt) },
    )
    #v(px(14))
    #line(length: 100%, stroke: px(1) + rgb(255, 255, 255, 18%))
    #v(px(10))
    #text(
      font: font-mono,
      size: tx(11),
      weight: 700,
      fill: color-rust-hi,
      tracking: 0.24em,
      "MARKDOWNREADER",
    )
  ]
]


#let divider() = block(width: 100%, above: px(34), below: px(34), line(
  length: 100%,
  stroke: (paint: color-hair, thickness: px(1), dash: "dashed"),
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
    radius: px(2),
    text(weight: 700, fill: color-rust, it.body),
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
      font: hf(3, font-serif),
      size: tx(size-h3),
      weight: 700,
      fill: color-rust,
    )
    #set par(leading: 0.42em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(
      font: hf(4, font-serif),
      size: tx(size-h4),
      weight: 700,
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
      font: hf(6, font-mono),
      size: tx(size-h6),
      weight: 700,
      fill: color-muted,
      tracking: 0.16em,
    ); #set par(leading: 0.42em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, block(
          stroke: px(2) + color-navy,
          inset: px(5),
          fill: white,
          {
            set image(width: w)
            it
          },
        ))
      })
    } else { it }
  }

  show link: it => underline(offset: px(3), stroke: px(1) + color-rust, text(
    fill: color-rust,
    weight: 600,
    it.body,
  ))

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(26),
    below: px(26),
    fill: color-chip,
    stroke: (left: px(4) + color-rust-hi, rest: none),
    inset: (left: px(22), rest: px(18)),
  )[
    #grid(
      columns: (auto, 1fr),
      column-gutter: px(8),
      align: horizon,
      text(
        font: font-bold,
        size: tx(34),
        weight: 700,
        fill: color-rust,
        tracking: 0em,
      )[“],
      text(
        font: font-mono,
        size: tx(11),
        weight: 700,
        fill: color-rust,
        tracking: 0.16em,
      )[QUOTE],
    )
    #v(px(6))
    #set text(size: tx(16), fill: color-soft, style: "italic")
    #it.body
  ]

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    weight: 600,
    fill: color-rust,
  )
  show raw.where(block: false): box.with(
    fill: color-chip,
    inset: (x: px(5), y: px(1)),
    radius: px(2),
    outset: (y: px(2)),
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-navy,
    stroke: px(2) + color-rust-hi,
    inset: px(18),
  )[
    #set par(leading: 0.5em, justify: false)
    #set text(font: font-mono, size: tx(size-code), fill: color-paper)
    #it
  ]

  set list(
    marker: text(font: font-mono, fill: color-rust, weight: 700)[—],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(
      width: px(22),
      height: px(22),
      fill: color-navy,
      align(center + horizon, text(
        font: font-mono,
        size: tx(13),
        weight: 700,
        fill: color-paper,
        str(n),
      )),
    ),
    body-indent: px(10),
    spacing: px(8),
  )

  set table(
    stroke: px(1) + color-table-bd,
    inset: (x: px(16), y: px(12)),
    fill: (_, row) => if row == 0 { color-paper } else if calc.even(row) {
      color-zebra
    } else { none },
  )
  show table: it => block(
    width: 100%,
    clip: true,
    stroke: px(2) + color-navy,
    breakable: true,
    it,
  )
  show table: set text(size: tx(15), fill: color-ink)
  show table.cell.where(y: 0): set text(
    font: font-mono,
    weight: 700,
    fill: color-rust,
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
