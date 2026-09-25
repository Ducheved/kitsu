"""Monthly statements: a text version for email and a CSV for accounting."""


def text_report(rows):
    """rows: [(label, cents)]. Amounts right-aligned, negatives in parentheses."""
    lines = []
    for label, cents in rows:
        units, rest = divmod(abs(cents), 100)
        amount = f"{units:,}.{rest:02d}"
        if cents < 0:
            amount = f"({amount})"
        lines.append(f"{label:<20}{amount:>16}")
    total = sum(cents for _, cents in rows)
    units, rest = divmod(abs(total), 100)
    amount = f"{units:,}.{rest:02d}"
    if total < 0:
        amount = f"({amount})"
    lines.append(f"{'Total':<20}{amount:>16}")
    return "\n".join(lines) + "\n"


def csv_report(rows):
    """rows: [(label, cents)]. Plain numbers for spreadsheets: no separators, a minus sign."""
    out = ["label,amount"]
    for label, cents in rows:
        units, rest = divmod(abs(cents), 100)
        sign = "-" if cents < 0 else ""
        out.append(f"{label},{sign}{units}.{rest:02d}")
    return "\n".join(out) + "\n"
