// ═══════════════════════════════════════════════════════════════════════
//
// ═══════════════════════════════════════════════════════════════════════


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
#let tx(n) = n * p-fscale * 1pt

#let size-h1      = float(sys.inputs.at("theme-size-h1",      default: "46"))
#let size-h2      = float(sys.inputs.at("theme-size-h2",      default: "36"))
#let size-h3      = float(sys.inputs.at("theme-size-h3",      default: "30"))
#let size-h4      = float(sys.inputs.at("theme-size-h4",      default: "26"))
#let size-h5      = float(sys.inputs.at("theme-size-h5",      default: "23"))
#let size-h6      = float(sys.inputs.at("theme-size-h6",      default: "20"))
#let size-body    = float(sys.inputs.at("theme-size-body",    default: "26"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "16"))
#let size-code    = float(sys.inputs.at("theme-size-code",    default: "18"))

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#d6d6d6"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#8f96a3"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#111418"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#000000"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#33383f"))
#let theme-primary = rgb(sys.inputs.at("theme-color-primary", default: "#6884c8"))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#6884c8"))



#let long-page-width = 540pt
#let margin-x   = 39pt
#let margin-top = 78pt
#let margin-bot = 39pt
#let long-page-min-height = 720pt

#let color-bg          = theme-bg
#let color-title       = rgb("#ffffff")
#let color-text        = theme-ink
#let color-meta-strong = rgb("#ffffff")
#let color-meta-light  = rgb("#a2a2a2")
#let color-link        = theme-primary
#let color-quote-bg    = theme-panel
#let color-code-bg     = theme-panel
#let color-table-bd    = rgb("#333333")
#let color-table-head  = theme-panel
#let color-divider     = rgb(255, 255, 255, 12%)
#let color-quote-border = rgb("#FFDABF")

#let font-serif = (sys.inputs.at("theme-font-body", default: "Songti SC"), "PingFang SC")
#let font-sans  = ("PingFang SC", "Helvetica Neue", "PingFang SC")
#let font-mono  = (sys.inputs.at("theme-font-mono", default: "DejaVu Sans Mono"), "Apple Color Emoji")
#let hf(n, base) = {
  let k = "theme-font-h" + str(n)
  if k in sys.inputs { (sys.inputs.at(k), ..base.slice(1)) } else { base }
}

#let logo-url = ""

// ═══════════════════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════════════════

#let poster-title(title) = block(above: 0pt, below: 60pt, width: 100%)[
  #set text(font: font-serif, size: tx(54), weight: 700, fill: color-title)
  #set par(leading: 0.4em)
  #title
]

#let md-toc() = context {
  let hs = query(heading.where(level: 2))
  if hs.len() == 0 { return }
  block(width: 100%, above: 20pt, below: 12pt)[
    #text(font: font-sans, size: tx(20), weight: "medium", fill: color-meta-light, tracking: 0.15em)[CONTENTS]
  ]
  block(width: 100%, below: 24pt)[
    #set par(leading: 0.6em)
    #stack(spacing: 16pt, ..hs.map(h => grid(
      columns: (auto, 1fr), column-gutter: 16pt, align: horizon,
      box(width: 22pt, height: 22pt, radius: 4pt, stroke: 2pt + color-divider),
      link(h.location(), text(font: font-serif, fill: color-text, weight: 600, size: tx(26), h.body)),
    )))
  ]
  block(above: 24pt, below: 0pt, line(length: 100%, stroke: 0.5pt + color-divider))
}

#let signature-row(author: "", date: "", logo: logo-url) = block(above: 58pt, width: 100%)[
  #grid(
    columns: (1fr, auto),
    align: (left + bottom, right + bottom),
    [
      #text(font: font-sans, size: tx(20), fill: color-meta-strong, weight: "medium")[#author] \
      #text(font: font-sans, size: tx(20), fill: color-meta-light, weight: "regular")[#date]
    ],
    if logo != "" { image(logo, width: 64pt, height: 64pt) },
  )
]


// ═══════════════════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════════════════

#let divider() = block(width: 100%, above: 40pt, below: 40pt,
    line(length: 100%, stroke: 0.5pt + color-divider))

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
        #set text(font: font-sans, size: tx(size-caption), fill: color-meta-light, tracking: 0pt)
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

  set text(font: font-serif, fill: color-text, size: tx(size-body), lang: "zh", weight: 400, tracking: 0.05em)
  set par(
    justify: if p-justify == "" { true } else { p-justify == "true" },
    leading: 0.8em * l-factor,
    spacing: 58pt,
    first-line-indent: (amount: if p-indent == "true" { 2em } else { 0pt }, all: false),
  )

  show strong: it => text(weight: 600, fill: color-text, it.body)
  show emph: it => text(style: "italic", fill: color-text, it.body)

  show heading.where(level: 1): it => block(above: 52pt, below: 29pt, width: 100%)[
    #set text(font: hf(1, font-serif), size: tx(size-h1), weight: 600, fill: color-title)
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 2): it => block(above: 52pt, below: 29pt, width: 100%)[
    #set text(font: hf(2, font-serif), size: tx(size-h2), weight: 600, fill: color-title)
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 3): it => block(above: 52pt, below: 29pt, width: 100%)[
    #set text(font: hf(3, font-serif), size: tx(size-h3), weight: 600, fill: color-title)
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 4): it => block(above: 52pt, below: 29pt, width: 100%)[
    #set text(font: hf(4, font-serif), size: tx(size-h4), weight: 600, fill: color-title)
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 5): it => block(above: 52pt, below: 29pt, width: 100%)[
    #set text(font: hf(5, font-serif), size: tx(size-h5), weight: 600, fill: color-title)
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 6): it => block(above: 52pt, below: 29pt, width: 100%)[
    #set text(font: hf(6, font-serif), size: tx(size-h6), weight: 600, fill: color-title)
    #set par(leading: 0.4em)
    #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let w = calc.min(measure(it).width, size.width)
        align(center, { set image(width: w); it })
      })
    } else { it }
  }

  show link: it => {
    set text(fill: color-link)
    underline(stroke: 0.6pt + color-link, offset: 2pt, it.body)
  }

  show quote.where(block: true): it => block(
    fill: color-quote-bg,
    radius: (left: 0pt, right: 10pt),
    stroke: (left: 1.63pt + color-quote-border),
    inset: (x: 26pt, y: 20pt),
    width: 100%,
    spacing: 20pt,
    {
      set text(size: tx(22), fill: color-text)
      it.body
    },
  )

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code))
  show raw.where(block: false): box.with(
    fill: color-code-bg, inset: (x: 6pt, y: 0pt), outset: (y: 5pt), radius: 3pt,
  )
  show raw.where(block: true): it => block(
    fill: color-code-bg, inset: 20pt, radius: 6pt, width: 100%, spacing: 20pt,
    text(font: font-mono, size: tx(size-code), it),
  )

  set table(
    stroke: 0.5pt + color-table-bd,
    inset: 13pt,
    fill: (_, row) => if row == 0 { color-table-head } else { none },
  )
  show table: it => box(radius: 13pt, clip: true, stroke: 0.5pt + color-table-bd, it)

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