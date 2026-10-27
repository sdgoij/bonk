#set page(paper: "a4", margin: (x: 2cm, y: 1.8cm))
#set text(size: 9.5pt)

// The renderer passes the real payload through `--input data=data.json`.
// The inline payload keeps the template compilable by editors and previewers.
#let data = if "data" in sys.inputs {
  json(sys.inputs.data)
} else {
  (
    number: "2026-0001",
    issue_date: "2026-10-27",
    due_date: "2026-11-26",
    currency: "EUR",
    seller: (name: "Seller BV", address: "Main 1, Amsterdam, NL", vat: "NL123456782B01"),
    customer: (name: "Customer BV", address: "Rotterdam, NL", vat: none),
    lines: ((name: "Consulting", qty: "2", price: "50", vat: "21", total: "100.00"),),
    net: "100.00",
    tax: "21.00",
    gross: "121.00",
    notes: none,
  )
}

#grid(columns: (1fr, 1fr), gutter: 1cm, [
  #text(size: 18pt, weight: "bold")[Invoice #data.number]
  #v(4pt)
  #data.issue_date
  #if data.due_date != none [#linebreak() Due #data.due_date]
], [
  #text(weight: "bold")[#data.seller.name]
  #linebreak()
  #data.seller.address
  #if data.seller.vat != none [#linebreak() VAT #data.seller.vat]
])

#v(1cm)
#text(weight: "bold")[Bill to]
#v(2pt)
#data.customer.name \
#data.customer.address \
#if data.customer.vat != none [VAT #data.customer.vat]

#v(1cm)
#table(
  columns: (1fr, 50pt, 70pt, 45pt, 75pt),
  stroke: 0.5pt,
  [*Item*], [*Qty*], [*Unit price*], [*VAT*], [*Total*],
  ..data.lines.map(line => ([#line.name], [#line.qty], [#line.price], [#line.vat], [#line.total])).flatten(),
)

#v(0.5cm)
#align(right)[
  Net #data.net #data.currency \
  VAT #data.tax #data.currency \
  #text(weight: "bold")[Total #data.gross #data.currency]
]

#if data.notes != none [
  #v(1cm)
  #text(size: 8.5pt)[#data.notes]
]
