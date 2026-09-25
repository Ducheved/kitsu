"""Monthly statements: a text version for email and a CSV for accounting."""

from money import format_amount


def text_report(rows):
    """rows: [(label, cents)]. Amounts right-aligned, negatives in parentheses."""
    lines = [f"{label:<20}{format_amount(cents, grouping=True, parens=True):>16}" for label, cents in rows]
    total = sum(cents for _, cents in rows)
    lines.append(f"{'Total':<20}{format_amount(total, grouping=True, parens=True):>16}")
    return "\n".join(lines) + "\n"


def csv_report(rows):
    """rows: [(label, cents)]. Plain numbers for spreadsheets: no separators, a minus sign."""
    out = ["label,amount"]
    for label, cents in rows:
        out.append(f"{label},{format_amount(cents, grouping=False, parens=False)}")
    return "\n".join(out) + "\n"
