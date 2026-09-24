// Every translation must say the same things as English: same keys, same
// {placeholders}, and every plural form the language's rules can select.
// The type check catches missing keys; it can't see inside the strings.
import assert from "node:assert/strict";
import { test } from "node:test";
import en from "../src/lib/i18n/en.ts";
import de from "../src/lib/i18n/de.ts";
import fr from "../src/lib/i18n/fr.ts";
import ja from "../src/lib/i18n/ja.ts";
import ru from "../src/lib/i18n/ru.ts";

type Msg = string | Record<string, string>;
const locales: Record<string, Record<string, Msg>> = { de, fr, ja, ru };

const holes = (m: Msg) => {
  const forms = typeof m === "string" ? [m] : Object.values(m);
  return new Set(forms.flatMap((f) => [...f.matchAll(/\{(\w+)\}/g)].map((x) => x[1])));
};

for (const [code, messages] of Object.entries(locales)) {
  test(`${code}: same keys as English`, () => {
    assert.deepEqual(Object.keys(messages).sort(), Object.keys(en).sort());
  });

  test(`${code}: same placeholders as English`, () => {
    for (const [key, source] of Object.entries(en as Record<string, Msg>)) {
      const want = holes(source);
      // A plural form may leave out {n} ("one question"), but never invent one.
      if (typeof source !== "string") want.add("n");
      const got = holes(messages[key]!);
      for (const h of got) assert.ok(want.has(h), `${code} ${key}: unknown {${h}}`);
      for (const h of holes(source)) {
        if (h === "n" && typeof source !== "string") continue;
        assert.ok(got.has(h), `${code} ${key}: drops {${h}}`);
      }
    }
  });

  test(`${code}: every plural form the language can pick`, () => {
    const categories = new Intl.PluralRules(code).resolvedOptions().pluralCategories;
    for (const [key, source] of Object.entries(en as Record<string, Msg>)) {
      if (typeof source === "string") {
        assert.equal(typeof messages[key], "string", `${code} ${key}: should be a plain string`);
        continue;
      }
      const m = messages[key] as Record<string, string>;
      assert.equal(typeof m, "object", `${code} ${key}: should have plural forms`);
      for (const c of categories) {
        if (c === "many" && code === "fr") continue; // falls back to `other`, same wording
        assert.ok(c in m || c === "other", `${code} ${key}: missing "${c}"`);
      }
      assert.ok("other" in m, `${code} ${key}: missing "other"`);
    }
  });
}
