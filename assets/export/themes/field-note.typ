// Field Note — a centered, collectible knowledge-journal layout.

#let paged = sys.inputs.at("paged", default: "false") == "true"
#let paper = sys.inputs.at("paper", default: "")

#let p-fscale = float(sys.inputs.at("font-scale", default: "1.0"))
#let p-layout-scale = float(sys.inputs.at("layout-scale", default: "1.0"))
#let p-orient = sys.inputs.at("orientation", default: "")
#let p-indent = sys.inputs.at("indent", default: "")
#let p-justify = sys.inputs.at("justify", default: "")
#let p-pagenum = sys.inputs.at("page-number", default: "")
#let p-footer = sys.inputs.at("footer-text", default: "")

#let m-factor = float(sys.inputs.at("margin-scale", default: "1.0"))
#let l-factor = float(sys.inputs.at("line-height-scale", default: "1.0"))
#let layout-unit = if paper != "" { 1.6 } else { 1.0 }
#let px(n) = n * layout-unit * 1pt * p-layout-scale
#let tx(n) = px(n) * p-fscale

#let size-h1 = float(sys.inputs.at("theme-size-h1", default: "38"))
#let size-h2 = float(sys.inputs.at("theme-size-h2", default: "24"))
#let size-h3 = float(sys.inputs.at("theme-size-h3", default: "22"))
#let size-h4 = float(sys.inputs.at("theme-size-h4", default: "18"))
#let size-h5 = float(sys.inputs.at("theme-size-h5", default: "16"))
#let size-h6 = float(sys.inputs.at("theme-size-h6", default: "14"))
#let size-body = float(sys.inputs.at("theme-size-body", default: "17"))
#let size-caption = float(sys.inputs.at("theme-size-caption", default: "11"))
#let size-code = float(sys.inputs.at("theme-size-code", default: "13"))

#let theme-ink = rgb(sys.inputs.at("theme-color-ink", default: "#17212B"))
#let theme-muted = rgb(sys.inputs.at("theme-color-muted", default: "#5E6670"))
#let theme-bg = rgb(sys.inputs.at("theme-color-bg", default: "#F7F2E8"))
#let theme-panel = rgb(sys.inputs.at("theme-color-panel", default: "#E9DECB"))
#let theme-border = rgb(sys.inputs.at("theme-color-border", default: "#D7CBB7"))
#let theme-primary = rgb(sys.inputs.at(
  "theme-color-primary",
  default: "#2042A0",
))
#let theme-accent = rgb(sys.inputs.at("theme-color-accent", default: "#E4573D"))

#let long-page-width = 540pt
#let margin-x = 38pt
#let margin-top = 34pt
#let margin-bot = 38pt
#let long-page-min-height = 720pt

#let color-bg = theme-bg
#let color-ink = theme-ink
#let color-muted = theme-muted
#let color-cobalt = theme-primary
#let color-vermilion = theme-accent
#let color-sand = theme-panel
#let color-border = theme-border
#let color-code = rgb(23, 33, 43, 7%)
#let color-table-head = rgb(32, 66, 160, 10%)
#let color-zebra = rgb(255, 255, 255, 34%)

#let font-display = (
  sys.inputs.at("theme-font-heading", default: "Helvetica Neue"),
  "PingFang SC",
  "Arial",
)
#let font-serif = (
  sys.inputs.at("theme-font-body", default: "Georgia"),
  "Songti SC",
  "Hiragino Mincho ProN",
  "PingFang SC",
)
#let font-mono = (
  sys.inputs.at("theme-font-mono", default: "Menlo"),
  "DejaVu Sans Mono",
  "PingFang SC",
  "Apple Color Emoji",
)
#let hf(n, base) = {
  let key = "theme-font-h" + str(n)
  if key in sys.inputs { (sys.inputs.at(key), ..base.slice(1)) } else { base }
}

#let logo-url = ""
#let section-counter = counter("field-note-section")

#let issue-medallion(number: "01", radius: px(18), fill: color-cobalt) = circle(
  radius: radius,
  fill: fill,
  align(center + horizon, text(
    font: font-display,
    size: radius * 1.08,
    weight: 700,
    fill: color-bg,
    number,
  )),
)

#let poster-title(title) = block(width: 100%, below: px(24))[
  #align(center)[
    #text(
      font: font-display,
      size: tx(14),
      weight: 700,
      tracking: 0.2em,
      fill: color-ink,
    )[ORG STUDIO]
    #linebreak()
    #text(
      font: font-display,
      size: tx(10),
      weight: 600,
      tracking: 0.25em,
      fill: color-muted,
    )[FIELD NOTE 01]
    #v(px(12))
    #issue-medallion()
    #v(px(10))
    #set text(
      font: hf(1, font-display),
      size: tx(size-h1 * if paper != "" { 3.0 } else { 0.95 }),
      weight: 800,
      tracking: -0.035em,
      fill: color-ink,
    )
    #set par(leading: 0.36em)
    #title
  ]
]

#let lead-heading(body) = block(width: 100%, above: 0pt, below: px(20))[
  #align(center, {
    set text(
      font: hf(2, font-serif),
      size: tx(size-h2 * if paper != "" { 1.25 } else { 0.9 }),
      weight: 500,
      fill: color-ink,
    )
    set par(leading: 0.42em)
    body
  })
]

#let section-heading(body) = {
  section-counter.step()
  context {
    let index = section-counter.get().first() + 1
    let number = if index < 10 { "0" + str(index) } else { str(index) }
    block(
      width: 100%,
      above: px(24),
      below: px(16),
      fill: color-sand,
      inset: (x: px(18), y: px(10)),
    )[
      #align(center, grid(
        columns: (auto, auto),
        column-gutter: px(10),
        align: horizon,
        issue-medallion(number: number, radius: px(18), fill: color-vermilion),
        text(
          font: hf(3, font-display),
          size: tx(size-h3 * if paper != "" { 1.2 } else { 0.9 }),
          weight: 750,
          fill: color-ink,
          body,
        ),
      ))
    ]
  }
}

#let md-toc() = context {
  let headings = query(heading.where(level: 3))
  if headings.len() == 0 { return }
  block(width: 100%, above: px(24), below: px(18))[
    #align(center, text(
      font: font-display,
      size: tx(10),
      weight: 700,
      tracking: 0.18em,
      fill: color-cobalt,
    )[CONTENTS])
    #v(px(10))
    #stack(spacing: px(9), ..headings.enumerate().map(((index, heading)) => grid(
      columns: (auto, 1fr, auto),
      column-gutter: px(10),
      align: horizon,
      text(
        font: font-display,
        size: tx(11),
        weight: 700,
        fill: color-vermilion,
        if index < 8 { "0" + str(index + 2) } else { str(index + 2) },
      ),
      link(heading.location(), text(
        font: font-serif,
        size: tx(14),
        fill: color-ink,
        heading.body,
      )),
      context text(
        font: font-display,
        size: tx(10),
        fill: color-muted,
        counter(page).at(heading.location()).first(),
      ),
    )))
  ]
}

#let signature-row(author: "", date: "", logo: logo-url) = block(
  width: 100%,
  above: px(34),
)[
  #align(center)[
    #text(
      font: font-display,
      size: tx(11),
      weight: 700,
      fill: color-ink,
      author,
    )
    #if date != "" [
      #linebreak()
      #text(
        font: font-display,
        size: tx(9),
        tracking: 0.1em,
        fill: color-muted,
        date,
      )
    ]
    #if logo != "" { image(logo, width: px(36), height: px(36)) }
  ]
]

#let divider() = align(center, block(above: px(30), below: px(30))[
  #circle(radius: px(3), fill: color-vermilion)
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
        #set text(
          font: font-display,
          size: tx(size-caption),
          fill: color-muted,
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
    font: font-serif,
    fill: color-ink,
    size: tx(size-body * if paper != "" { 1.14 } else { 0.94 }),
    lang: "zh",
    weight: 400,
  )
  set par(
    justify: if p-justify == "" { false } else { p-justify == "true" },
    leading: 0.72em * l-factor,
    spacing: px(16),
    first-line-indent: (
      amount: if p-indent == "true" { 2em } else { 0pt },
      all: false,
    ),
  )
  set list(
    marker: text(font: font-display, weight: 700, fill: color-vermilion)[•],
    indent: px(16),
    body-indent: px(10),
    spacing: px(5),
  )
  set enum(
    numbering: "1.",
    number-align: right,
    indent: px(20),
    body-indent: px(10),
    spacing: px(5),
  )

  show strong: it => text(font: font-serif, weight: 700, fill: color-ink, it.body)
  show emph: it => text(font: font-serif, style: "italic", fill: color-muted, it.body)

  show heading: set par(justify: false)
  show heading.where(level: 1): it => lead-heading(it.body)
  show heading.where(level: 2): it => lead-heading(it.body)
  show heading.where(level: 3): it => section-heading(it.body)
  show heading.where(level: 4): it => block(above: px(24), below: px(11))[
    #set text(
      font: hf(4, font-display),
      size: tx(size-h4),
      weight: 700,
      fill: color-cobalt,
    )
    #set par(leading: 0.4em)
    #align(center, it.body)
  ]
  show heading.where(level: 5): it => block(above: px(20), below: px(9))[
    #set text(
      font: hf(5, font-display),
      size: tx(size-h5),
      weight: 700,
      fill: color-ink,
    )
    #set par(leading: 0.4em)
    #it.body
  ]
  show heading.where(level: 6): it => block(above: px(18), below: px(8))[
    #set text(
      font: hf(6, font-display),
      size: tx(size-h6),
      weight: 700,
      fill: color-muted,
    )
    #set par(leading: 0.4em)
    #it.body
  ]

  show image: it => {
    if it.width == auto {
      layout(size => {
        let width = calc.min(measure(it).width, size.width)
        align(center, {
          set image(width: width)
          it
        })
      })
    } else { it }
  }

  show link: it => text(fill: color-cobalt, weight: 600, it.body)

  show quote.where(block: true): set par(justify: false)
  show table.cell: set par(justify: false)

  show quote.where(block: true): it => block(
    width: 100%,
    above: px(22),
    below: px(22),
    inset: (x: px(16), y: px(9)),
  )[
    #align(center, {
      set text(
        font: hf(2, font-serif),
        size: tx(size-h2 * if paper != "" { 1.2 } else { 1.0 }),
        weight: 700,
        fill: color-cobalt,
      )
      set par(leading: 0.48em)
      it.body
    })
  ]

  show raw.where(block: false): set text(font: font-mono, size: tx(size-code))
  show raw.where(block: false): box.with(
    fill: color-code,
    inset: (x: px(5), y: px(1)),
    outset: (y: px(2)),
    radius: px(3),
  )
  show raw.where(block: true): it => block(
    width: 100%,
    fill: color-code,
    stroke: px(1) + color-border,
    inset: px(16),
    spacing: px(16),
    text(font: font-mono, size: tx(size-code), fill: color-ink, it),
  )

  set table(
    stroke: px(1) + color-border,
    inset: px(9),
    fill: (_, row) => if row == 0 {
      color-table-head
    } else if calc.odd(row) {
      color-zebra
    } else {
      none
    },
  )
  show table: it => block(width: 100%, breakable: true, it)

  show math.equation: set text(font: ("New Computer Modern Math",))

  if paged or paper != "" {
    doc
  } else {
    context {
      let min-inner = long-page-min-height - margin-top - margin-bot
      let height = measure(doc).height
      if height < min-inner { block(height: min-inner, doc) } else { doc }
    }
  }
}
