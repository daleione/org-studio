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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#e5e2e1"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#baccb0"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#131313"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#1c1b1b"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#353535"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#79ff5b",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#00f6f6"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "26"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "22"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "22"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "19"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "17"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "16"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "17"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "14"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let long-page-width = float(sys.inputs.at("long-page-width-pt", default: "540")) * 1pt
#let margin-x = float(sys.inputs.at("long-page-margin-x-pt", default: "39")) * 1pt
#let margin-top = 60pt
#let margin-bot = 48pt
#let long-page-min-height = 720pt

#let logo-url = ""

#let font-text = (
  sys.inputs.at("theme-font-body", default: "Menlo"),
  "PingFang SC",
)
#let font-bold = (
  sys.inputs.at("theme-font-heading", default: "Menlo"),
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
#let color-panel = theme-panel
#let color-panel2 = rgb("#0a0a0a")
#let color-bar = theme-border
#let color-green = rgb("#efffe3")
#let color-neon = theme-primary
#let color-neon-dim = rgb("#2ae500")
#let color-magenta = rgb("#ffabf3")
#let color-cyan = theme-accent
#let color-ink = theme-ink
#let color-soft = theme-muted
#let color-muted = rgb(186, 204, 176, 60%)
#let color-shadow = rgb("#000000")
#let color-zebra = rgb(53, 53, 53, 22%)
#let color-chip = rgb(121, 255, 91, 14%)

#let pixel-panel(
  body,
  fill: color-panel,
  border: color-bar,
  inset: px(18),
  above: px(24),
  below: px(24),
) = block(
  width: 100%,
  above: above,
  below: below,
  fill: color-shadow,
)[
  #block(
    width: 100%,
    fill: fill,
    stroke: px(2) + border,
    inset: inset,
    outset: (bottom: 0pt, right: 0pt),
    radius: 0pt,
    body,
  )
]

#let pixel-shadow-box(
  body,
  fill: color-panel,
  border: color-bar,
  inset: px(18),
  above: px(24),
  below: px(24),
) = block(
  width: 100%,
  above: above,
  below: below,
)[
  #box(width: 100%, inset: (right: px(4), bottom: px(4)))[
    #box(width: 100%, fill: color-shadow)[
      #move(dx: -px(4), dy: -px(4), block(
        width: 100%,
        fill: fill,
        stroke: px(3) + border,
        radius: 0pt,
        inset: inset,
        body,
      ))
    ]
  ]
]

#let hairline(above: px(0), below: px(0)) = block(
  width: 100%,
  above: above,
  below: below,
  line(length: 100%, stroke: (
    paint: color-bar,
    thickness: px(2),
    dash: ("dot", px(3)),
  )),
)

#let h1-block(body) = block(width: 100%, above: px(34), below: px(20))[
  #text(
    font: font-text,
    size: tx(15),
    weight: 500,
    fill: color-neon-dim,
    tracking: 0.18em,
  )[>> SYSTEM]
  #v(px(6))
  #grid(
    columns: (auto, 1fr),
    column-gutter: px(10),
    align: top,
    text(font: font-bold, size: tx(34), weight: 500, fill: color-neon-dim, "#"),
    {
      set text(
        font: hf(1, font-bold),
        size: tx(size-h1),
        weight: 700,
        fill: color-green,
        tracking: 0.02em,
      )
      set par(leading: 0.4em, justify: false)
      body
    },
  )
  #v(px(14))
  #line(length: 100%, stroke: (
    paint: color-neon-dim,
    thickness: px(2),
    dash: ("dot", px(3)),
  ))
]

#let h2-block(body) = block(width: 100%, above: px(32), below: px(16))[
  #box(
    width: 100%,
    fill: color-bar,
    inset: (x: px(14), y: px(8)),
    radius: 0pt,
    stroke: (left: px(5) + color-neon),
  )[
    #grid(
      columns: (auto, 1fr, auto),
      column-gutter: px(8),
      align: horizon,
      text(font: font-bold, size: tx(22), weight: 500, fill: color-neon, "["),
      {
        set text(
          font: hf(2, font-bold),
          size: tx(size-h2),
          weight: 500,
          fill: color-green,
          tracking: 0.04em,
        )
        set par(leading: 0.4em)
        body
      },
      text(font: font-bold, size: tx(22), weight: 500, fill: color-neon, "]"),
    )
  ]
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: px(20), below: px(8))[
    #text(
      font: font-text,
      size: tx(15),
      weight: 500,
      fill: color-magenta,
      tracking: 0.2em,
      "[## CONTENTS · CONTENTS]",
    )
  ]
  pixel-shadow-box(
    border: color-neon,
    fill: color-panel2,
    below: px(16),
    inset: px(18),
  )[
    #block(
      width: 100%,
      fill: color-neon,
      inset: (x: px(10), y: px(5)),
      radius: 0pt,
    )[
      #text(
        font: font-text,
        size: tx(15),
        weight: 500,
        fill: color-shadow,
        tracking: 0.16em,
      )[DIRECTORY/INDEX.SYS]
    ]
    #v(px(12))
    #set par(leading: 0.5em)
    #stack(spacing: px(12), ..hs
      .enumerate()
      .map(((i, h)) => grid(
        columns: (auto, 1fr),
        column-gutter: px(10),
        align: horizon,
        text(
          font: font-mono,
          size: tx(15),
          fill: color-neon,
          weight: 700,
          std.numbering("01.", i + 1),
        ),
        link(h.location(), text(
          font: font-text,
          fill: color-soft,
          weight: 500,
          size: tx(18),
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
  #hairline(below: px(16))
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: px(12),
    align: (horizon, left + horizon, right + horizon),
    box(fill: color-neon, inset: 0pt, radius: 0pt, box(
      width: px(40),
      height: px(40),
    )[
      #align(center + horizon, text(
        font: font-bold,
        size: tx(24),
        weight: 500,
        fill: color-shadow,
        if author != "" { author.first() } else { "" },
      ))
    ]),
    [
      #text(
        font: font-text,
        size: tx(18),
        fill: color-green,
        weight: 500,
      )[#author] \
      #text(
        font: font-text,
        size: tx(15),
        fill: color-muted,
        tracking: 0.1em,
      )[#date]
    ],
    if logo != "" { image(logo, width: 50pt, height: 50pt) },
  )
]


#let divider() = block(width: 100%, above: px(34), below: px(34), line(
  length: 100%,
  stroke: (paint: color-bar, thickness: px(2), dash: ("dot", px(3))),
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
    weight: 500,
    tracking: 0.01em,
  )
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.72em * l-factor,
    spacing: px(18),
    first-line-indent: (
      amount: if p-indent == "true" { 2em } else { 0pt },
      all: false,
    ),
  )

  show strong: it => highlight(
    fill: color-neon,
    top-edge: 0.32em,
    bottom-edge: -0.2em,
    extent: px(2),
    radius: 0pt,
    text(weight: 500, fill: color-shadow, it.body),
  )
  show emph: it => text(style: "italic", fill: color-magenta, it.body)

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
      weight: 500,
      fill: color-neon-dim,
      tracking: 0.04em,
    )
    #set par(leading: 0.4em)
    #text(fill: color-magenta, "### ") #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(10))[
    #set text(
      font: hf(4, font-bold),
      size: tx(size-h4),
      weight: 500,
      fill: color-green,
    ); #set par(leading: 0.4em)
    #text(fill: color-magenta, "#### ") #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(
      font: hf(5, font-bold),
      size: tx(size-h5),
      weight: 500,
      fill: color-soft,
    ); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(
      font: hf(6, font-bold),
      size: tx(size-h6),
      weight: 500,
      fill: color-muted,
    ); #set par(leading: 0.4em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, box(fill: color-shadow)[
          #move(dx: -px(4), dy: -px(4), block(
            stroke: px(3) + color-bar,
            radius: 0pt,
            clip: true,
            inset: px(4),
            fill: color-panel2,
            {
              set image(width: w - px(8))
              it
            },
          ))
        ])
      })
    } else { it }
  }

  show link: it => text(fill: color-cyan, weight: 500, it.body)

  show quote.where(block: true): set par(justify: false)

  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(26),
    below: px(26),
    fill: color-panel2,
    stroke: (left: px(5) + color-neon-dim, rest: px(2) + color-bar),
    radius: 0pt,
    inset: (left: px(20), rest: px(16)),
  )[
    #text(
      font: font-text,
      size: tx(14),
      weight: 500,
      fill: color-neon-dim,
      tracking: 0.16em,
      "// QUOTE",
    )
    #v(px(8))
    #set text(size: tx(18), fill: color-soft, style: "italic")
    #it.body
  ]

  show raw.where(block: false): set text(
    font: font-mono,
    size: tx(size-code),
    fill: color-neon,
  )
  show raw.where(block: false): box.with(
    fill: color-chip,
    inset: (x: px(5), y: px(1)),
    radius: 0pt,
    outset: (y: px(2)),
    stroke: px(1) + color-bar,
  )
  show raw.where(block: true): it => block(
    width: 100%,
    above: px(24),
    below: px(24),
    fill: color-shadow,
  )[
    #move(dx: -px(4), dy: -px(4))[
      #block(
        width: 100%,
        fill: color-panel2,
        stroke: px(3) + color-bar,
        radius: 0pt,
        inset: 0pt,
      )[
        #block(
          width: 100%,
          fill: color-bar,
          inset: (x: px(12), y: px(7)),
          radius: 0pt,
        )[
          #grid(
            columns: (1fr, auto),
            align: horizon,
            text(
              font: font-mono,
              size: tx(12),
              fill: color-soft,
              tracking: 0.1em,
            )[TERMINAL.SH],
            stack(
              dir: ltr,
              spacing: px(5),
              box(width: px(9), height: px(9), fill: rgb("#ffb4ab")),
              box(width: px(9), height: px(9), fill: color-magenta),
              box(width: px(9), height: px(9), fill: color-neon),
            ),
          )
        ]
        #block(width: 100%, inset: px(16))[
          #set par(leading: 0.5em, justify: false)
          #set text(font: font-mono, size: tx(size-code), fill: color-neon)
          #it
        ]
      ]
    ]
  ]

  set list(
    marker: text(font: font-mono, fill: color-neon, weight: 700)[■],
    body-indent: px(8),
    spacing: px(8),
  )
  set enum(
    numbering: n => box(inset: (right: px(2)), text(
      font: font-mono,
      size: tx(15),
      weight: 700,
      fill: color-neon-dim,
      std.numbering("01.", n),
    )),
    body-indent: px(8),
    spacing: px(8),
  )

  set table(
    stroke: px(2) + color-bar,
    inset: (x: px(14), y: px(11)),
    fill: (_, row) => if row == 0 { color-bar } else if calc.odd(row) {
      color-zebra
    } else { color-panel2 },
  )
  show table: it => block(width: 100%, fill: color-shadow)[
    #move(dx: -px(4), dy: -px(4), block(
      width: 100%,
      radius: 0pt,
      clip: true,
      stroke: px(3) + color-bar,
      breakable: true,
      it,
    ))
  ]
  show table: set text(font: font-mono, size: tx(14), fill: color-soft)
  show table.cell.where(y: 0): set text(weight: 700, fill: color-neon)

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
