+++
title = "What VAT rate does finance want for Norway (NO)?"
blocks = ["norway-invoices"]
+++
Invoicing Norwegian customers fails because `rates.csv` has no `NO` row.
Decision `rates-from-finance` says rates come only from finance's signed
table, so I haven't added one. Once finance adds `NO` to `rates.csv`,
invoices work with no code change.
