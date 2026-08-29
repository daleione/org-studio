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

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#333333"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#666666"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#ffffff"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#faf8f5"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#e0e0e0"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#8b1e22"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#8b1e22"))



#let px(n) = n * 1.6pt * p-layout-scale
#let tx(n) = px(n) * p-fscale
#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "22"))
#let size-h2      = float(sys.inputs.at("theme-size-h2",      default: "20"))
#let size-h3      = float(sys.inputs.at("theme-size-h3",      default: "18"))
#let size-h4      = float(sys.inputs.at("theme-size-h4",      default: "16"))
#let size-h5      = float(sys.inputs.at("theme-size-h5",      default: "15"))
#let size-h6      = float(sys.inputs.at("theme-size-h6",      default: "14"))
#let size-body    = float(sys.inputs.at("theme-size-body",    default: "15"))
#let size-code    = float(sys.inputs.at("theme-size-code",    default: "14"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "12"))
#let long-page-width      = 540pt
#let margin-x        = 39pt
#let margin-top      = 60pt
#let margin-bot      = 48pt
#let long-page-min-height = 720pt
#let logo-url = ""

#let font-text  = (sys.inputs.at("theme-font-body", default: "Helvetica Neue"), "PingFang SC")
#let font-bold  = (sys.inputs.at("theme-font-heading", default: "Helvetica Neue"), "PingFang SC", "PingFang SC")
#let font-mono  = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "PingFang SC", "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let color-bg       = theme-bg
#let color-crimson  = theme-primary
#let color-text     = theme-muted
#let color-li       = theme-ink
#let color-em       = rgb("#555555")
#let color-panel    = theme-panel
#let color-zebra    = rgb("#f9f9f9")

#let h1-block(body) = align(center, block(
  width: 80%, above: px(24), below: px(24),
  fill: rgb(139, 30, 34, 3%),
  stroke: (top: px(2) + color-crimson, bottom: px(2) + color-crimson),
  inset: (x: px(24), y: px(12)),
)[
  #set text(font: hf(1, font-bold), size: tx(size-h1), weight: 700, fill: color-crimson, tracking: 0.1em)
  #set par(leading: 0.4em)
  #align(center, body)
])

#let h2-block(body) = block(
  width: 100%, above: px(32), below: px(12),
  stroke: (
    left: px(4) + color-crimson,
    bottom: (paint: rgb(139, 30, 34, 40%), thickness: px(1), dash: "dashed"),
  ),
  inset: (left: px(12), bottom: px(8), top: 2pt),
)[
  #set text(font: hf(2, font-bold), size: tx(size-h2), weight: 700, fill: color-crimson, tracking: 0.1em)
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
      columns: (auto, 1fr), column-gutter: px(8),
      text(fill: color-crimson, weight: 700, size: tx(15))[•],
      link(h.location(), text(fill: color-crimson, size: tx(15), h.body)),
    )))
  ]
}

#let poster-title(title) = h1-block(title)

#let signature-row(author: "", date: "", logo: logo-url) = block(above: px(34), width: 100%)[
  #grid(
    columns: (1fr, auto),
    align: (left + bottom, right + bottom),
    [
      #text(font: font-text, size: tx(15), fill: color-li, weight: 600)[#author] \
      #text(font: font-text, size: tx(15), fill: color-text)[#date]
    ],
    if logo != "" { image(logo, width: 56pt, height: 56pt) },
  )
]


#let divider() = align(center, block(
    width: 80%, height: px(1), above: px(40), below: px(40),
    fill: gradient.linear(
      (rgb(139, 30, 34, 0%), 0%), (rgb(139, 30, 34, 50%), 50%),
      (rgb(139, 30, 34, 0%), 100%), dir: ltr),
  ))

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
  set text(font: font-text, fill: color-text, size: tx(size-body), lang: "zh", weight: 400)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 1em * l-factor,
    spacing: px(19),
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => text(weight: 600, fill: color-crimson, it.body)
  show emph: it => text(style: "italic", fill: color-em, it.body)

  show heading.where(level: 1): it => h1-block(it.body)
  show heading.where(level: 2): it => h2-block(it.body)
  show heading.where(level: 3): it => block(
    width: 100%, above: px(32), below: px(12),
    stroke: (left: px(3) + color-crimson), inset: (left: px(12), bottom: px(6), top: 2pt),
  )[
    #set text(font: hf(3, font-bold), size: tx(size-h3), weight: 600, fill: color-crimson)
    #set par(leading: 0.5em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: px(24), below: px(12))[
    #set text(font: hf(4, font-text), size: tx(size-h4), weight: 600, fill: color-crimson); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(10))[
    #set text(font: hf(5, font-text), size: tx(size-h5), weight: 600, fill: color-em); #set par(leading: 0.4em); #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(10))[
    #set text(font: hf(6, font-text), size: tx(size-h6), weight: 600, fill: color-em); #set par(leading: 0.4em); #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, { set image(width: w); it })
      })
    } else { it }
  }

  show link: it => text(fill: color-crimson, it.body)

  show quote.where(block: true): it => block(
    width: 100%, above: px(19), below: px(19),
    fill: rgb(139, 30, 34, 3%),
    stroke: (left: px(4) + color-crimson),
    inset: (left: px(16), right: px(16), y: px(8)),
    { set text(size: tx(15), fill: color-text); it.body },
  )

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code), fill: color-li)
  show raw.where(block: true): it => block(
    width: 100%, above: px(24), below: px(24),
    fill: color-panel, radius: px(8), clip: true,
    stroke: (top: px(3) + color-crimson),
  )[
    #block(width: 100%, height: px(36), fill: rgb("#f0ece5"), inset: (x: px(12)))[
      #align(horizon, stack(dir: ltr, spacing: px(8),
        circle(radius: px(6), fill: rgb("#FF5F56")),
        circle(radius: px(6), fill: rgb("#FFBD2E")),
        circle(radius: px(6), fill: rgb("#27C93F")),
      ))
    ]
    #block(width: 100%, inset: px(19))[
      #set par(leading: 0.45em, justify: false)
      #set text(font: font-mono, size: tx(size-code), fill: color-li)
      #it
    ]
  ]

  set list(marker: text(fill: color-crimson, weight: 700)[•], body-indent: px(8), spacing: px(8))
  set enum(
    numbering: n => box(
      width: px(22), height: px(22), radius: px(11), fill: rgb(139, 30, 34, 10%),
      align(center + horizon, text(size: tx(13), weight: 700, fill: color-crimson, str(n))),
    ),
    body-indent: px(10), spacing: px(8),
  )

  set table(
    stroke: (_, row) => if row == 0 {
      (bottom: px(2) + rgb(139, 30, 34, 50%))
    } else {
      (bottom: px(1) + rgb(139, 30, 34, 10%))
    },
    inset: px(12),
    fill: (_, row) => if row == 0 {
      gradient.linear(rgb(139, 30, 34, 15%), rgb(139, 30, 34, 5%), angle: 90deg)
    } else if calc.even(row) { color-zebra } else { none },
  )
  show table: it => block(width: 100%, stroke: px(1) + rgb(139, 30, 34, 30%), breakable: true, it)
  show table: set text(size: tx(15), fill: color-li)
  show table.cell.where(y: 0): set text(fill: color-crimson, weight: 600)

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