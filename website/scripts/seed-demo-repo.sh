#!/usr/bin/env bash
# Seeds a believable demo repo for the landing screenshots (see website/screenshot-plan.md).
# Usage: ./seed-demo-repo.sh [target-dir]   (default: ~/demo/checkout)
set -euo pipefail

TARGET="${1:-$HOME/demo/checkout}"

if [ -e "$TARGET" ]; then
  echo "error: $TARGET already exists, remove it first" >&2
  exit 1
fi

AVA_NAME="Ava Collins"
AVA_MAIL="ava@lumenware.dev"
NOAH_NAME="Noah Fischer"
NOAH_MAIL="noah@lumenware.dev"

mkdir -p "$TARGET"
cd "$TARGET"
git init -q -b main
git config user.name "$AVA_NAME"
git config user.email "$AVA_MAIL"

commit() {
  local message="$1" name="$2" mail="$3" date="$4"
  git add -A
  GIT_AUTHOR_NAME="$name" GIT_AUTHOR_EMAIL="$mail" GIT_AUTHOR_DATE="$date" \
  GIT_COMMITTER_NAME="$name" GIT_COMMITTER_EMAIL="$mail" GIT_COMMITTER_DATE="$date" \
    git commit -q -m "$message"
}

# --- main history -----------------------------------------------------------

cat > package.json <<'EOF'
{
  "name": "@lumenware/checkout",
  "version": "0.4.2",
  "description": "Cart, pricing, taxes and discounts for the Lumenware storefront",
  "type": "module",
  "main": "src/index.ts",
  "scripts": {
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": {
    "typescript": "^5.6.0",
    "vitest": "^2.1.0"
  }
}
EOF

cat > tsconfig.json <<'EOF'
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "skipLibCheck": true
  },
  "include": ["src", "test"]
}
EOF

cat > README.md <<'EOF'
# @lumenware/checkout

Cart, pricing, taxes and discounts for the Lumenware storefront.

```ts
import { createCart, addItem, cartTotal } from "@lumenware/checkout";

const cart = addItem(createCart("EUR"), { sku: "LW-DESK-01", unitPrice: 74900, quantity: 1 });
cartTotal(cart); // { subtotal: 74900, tax: 14980, total: 89880 }
```
EOF

printf 'node_modules/\ndist/\n' > .gitignore

mkdir -p src test

cat > src/index.ts <<'EOF'
export * from "./cart";
export * from "./pricing";
export * from "./tax";
export * from "./discounts";
EOF

commit "chore: scaffold checkout package" "$AVA_NAME" "$AVA_MAIL" "2026-08-10T09:12:00"

cat > src/cart.ts <<'EOF'
export interface LineItem {
  sku: string;
  unitPrice: number;
  quantity: number;
}

export interface Cart {
  currency: string;
  items: LineItem[];
}

export function createCart(currency: string): Cart {
  return { currency, items: [] };
}

export function addItem(cart: Cart, item: LineItem): Cart {
  const existing = cart.items.find((line) => line.sku === item.sku);
  if (!existing) {
    return { ...cart, items: [...cart.items, item] };
  }
  const items = cart.items.map((line) =>
    line.sku === item.sku
      ? { ...line, quantity: line.quantity + item.quantity }
      : line,
  );
  return { ...cart, items };
}

export function subtotal(cart: Cart): number {
  return cart.items.reduce(
    (sum, line) => sum + line.unitPrice * line.quantity,
    0,
  );
}
EOF

commit "feat: cart model with line items" "$AVA_NAME" "$AVA_MAIL" "2026-08-10T14:37:00"

cat > src/pricing.ts <<'EOF'
const ZERO_DECIMAL_CURRENCIES = new Set(["JPY", "KRW", "VND"]);

export function roundCurrency(amount: number, currency: string): number {
  if (ZERO_DECIMAL_CURRENCIES.has(currency)) {
    return Math.round(amount / 100) * 100;
  }
  return Math.round(amount);
}

export function unitTotal(
  unitPrice: number,
  quantity: number,
  currency: string,
): number {
  return roundCurrency(unitPrice * quantity, currency);
}
EOF

commit "feat: pricing with per-currency rounding" "$NOAH_NAME" "$NOAH_MAIL" "2026-08-11T10:05:00"

cat > src/tax.ts <<'EOF'
export interface TaxRegion {
  code: string;
  rate: number;
}

const REGIONS: TaxRegion[] = [
  { code: "EU", rate: 0.2 },
  { code: "US-CA", rate: 0.0725 },
  { code: "UK", rate: 0.2 },
  { code: "CH", rate: 0.081 },
];

export function taxRate(regionCode: string): number {
  const region = REGIONS.find((entry) => entry.code === regionCode);
  return region ? region.rate : 0;
}

export function taxAmount(subtotal: number, regionCode: string): number {
  return subtotal * taxRate(regionCode);
}
EOF

commit "feat: tax rates per region" "$AVA_NAME" "$AVA_MAIL" "2026-08-12T16:48:00"

cat > test/cart.test.ts <<'EOF'
import { describe, expect, it } from "vitest";
import { addItem, createCart, subtotal } from "../src/cart";

describe("cart", () => {
  it("accumulates quantity for the same sku", () => {
    let cart = createCart("EUR");
    cart = addItem(cart, { sku: "LW-DESK-01", unitPrice: 74900, quantity: 1 });
    cart = addItem(cart, { sku: "LW-DESK-01", unitPrice: 74900, quantity: 2 });
    expect(cart.items).toHaveLength(1);
    expect(subtotal(cart)).toBe(224700);
  });

  it("keeps distinct skus as separate lines", () => {
    let cart = createCart("EUR");
    cart = addItem(cart, { sku: "LW-DESK-01", unitPrice: 74900, quantity: 1 });
    cart = addItem(cart, { sku: "LW-LAMP-02", unitPrice: 12900, quantity: 1 });
    expect(cart.items).toHaveLength(2);
  });
});
EOF

commit "test: cart totals" "$NOAH_NAME" "$NOAH_MAIL" "2026-08-13T11:22:00"

BRANCH_POINT=$(git rev-parse HEAD)

cat > src/pricing.ts <<'EOF'
const ZERO_DECIMAL_CURRENCIES = new Set(["JPY", "KRW", "VND"]);

export function roundCurrency(amount: number, currency: string): number {
  if (ZERO_DECIMAL_CURRENCIES.has(currency)) {
    return Math.round(amount / 100) * 100;
  }
  // Half-even keeps totals stable when summing many small lines.
  const floor = Math.floor(amount);
  const diff = amount - floor;
  if (diff > 0.5) return floor + 1;
  if (diff < 0.5) return floor;
  return floor % 2 === 0 ? floor : floor + 1;
}

export function unitTotal(
  unitPrice: number,
  quantity: number,
  currency: string,
): number {
  if (quantity <= 0) return 0;
  return roundCurrency(unitPrice * quantity, currency);
}
EOF

commit "fix: half-even rounding and zero-quantity lines" "$AVA_NAME" "$AVA_MAIL" "2026-08-15T09:40:00"

cat > src/discounts.ts <<'EOF'
export interface FlatDiscount {
  kind: "flat";
  amount: number;
}

export type Discount = FlatDiscount;

export function applyDiscounts(subtotal: number, discounts: Discount[]): number {
  const total = discounts.reduce((sum, discount) => sum + discount.amount, 0);
  return Math.max(0, subtotal - total);
}
EOF

commit "feat: flat discounts" "$NOAH_NAME" "$NOAH_MAIL" "2026-08-18T15:03:00"

cat > src/index.ts <<'EOF'
export * from "./cart";
export * from "./pricing";
export * from "./tax";
export * from "./discounts";

export { cartTotal } from "./totals";
EOF

cat > src/totals.ts <<'EOF'
import { type Cart, subtotal } from "./cart";
import { type Discount, applyDiscounts } from "./discounts";
import { roundCurrency } from "./pricing";
import { taxAmount } from "./tax";

export interface CartTotal {
  subtotal: number;
  tax: number;
  total: number;
}

export function cartTotal(
  cart: Cart,
  regionCode = "EU",
  discounts: Discount[] = [],
): CartTotal {
  const beforeDiscount = subtotal(cart);
  const discounted = applyDiscounts(beforeDiscount, discounts);
  const tax = roundCurrency(taxAmount(discounted, regionCode), cart.currency);
  return {
    subtotal: beforeDiscount,
    tax,
    total: discounted + tax,
  };
}
EOF

commit "feat: cart totals with taxes and discounts" "$AVA_NAME" "$AVA_MAIL" "2026-08-20T10:55:00"

# --- feature/tax-rounding: conflicts with main on pricing.ts ----------------

git checkout -q -b feature/tax-rounding "$BRANCH_POINT"

cat > src/pricing.ts <<'EOF'
const ZERO_DECIMAL_CURRENCIES = new Set(["JPY", "KRW", "VND"]);
const FIVE_CENT_CURRENCIES = new Set(["CHF"]);

export function roundCurrency(amount: number, currency: string): number {
  if (ZERO_DECIMAL_CURRENCIES.has(currency)) {
    return Math.round(amount / 100) * 100;
  }
  if (FIVE_CENT_CURRENCIES.has(currency)) {
    return Math.round(amount / 5) * 5;
  }
  return Math.round(amount);
}

export function unitTotal(
  unitPrice: number,
  quantity: number,
  currency: string,
): number {
  return roundCurrency(unitPrice * quantity, currency);
}
EOF

commit "feat: CHF five-cent cash rounding" "$NOAH_NAME" "$NOAH_MAIL" "2026-08-16T13:27:00"

# --- feature/messy-history: interactive rebase material ---------------------

git checkout -q main
git checkout -q -b feature/messy-history

printf '\nexport const MAX_STACKED_DISCOUNTS = 3;\n' >> src/discounts.ts
commit "wip stacking" "$AVA_NAME" "$AVA_MAIL" "2026-08-21T09:14:00"

sed -i.bak 's/MAX_STACKED_DISCOUNTS = 3/MAX_STACKED_DISCOUNTS = 4/' src/discounts.ts && rm src/discounts.ts.bak
commit "wip stacking again" "$AVA_NAME" "$AVA_MAIL" "2026-08-21T09:41:00"

sed -i.bak 's/Cart, pricing, taxes/Cart, pricing, taxes,/' README.md && rm README.md.bak
commit "fix typo" "$AVA_NAME" "$AVA_MAIL" "2026-08-21T10:02:00"

cat > test/discounts.test.ts <<'EOF'
import { describe, expect, it } from "vitest";
import { applyDiscounts } from "../src/discounts";

describe("discounts", () => {
  it("never goes below zero", () => {
    expect(applyDiscounts(1000, [{ kind: "flat", amount: 2500 }])).toBe(0);
  });
});
EOF
commit "test: flat discount clamping" "$AVA_NAME" "$AVA_MAIL" "2026-08-21T10:30:00"

sed -i.bak 's/MAX_STACKED_DISCOUNTS = 4/MAX_STACKED_DISCOUNTS = 3/' src/discounts.ts && rm src/discounts.ts.bak
commit "revert stacking limit to 3" "$AVA_NAME" "$AVA_MAIL" "2026-08-21T11:12:00"

# --- feature/checkout-discounts: the hero working tree ----------------------

git checkout -q main
git checkout -q -b feature/checkout-discounts

# a stash entry so the palette has something to show
printf '\nexport const EXPERIMENTAL_TIERED_TAX = false;\n' >> src/tax.ts
git stash push -q -m "experiment: tiered tax flag"

cat > src/discounts.ts <<'EOF'
export interface FlatDiscount {
  kind: "flat";
  amount: number;
}

export interface PercentageDiscount {
  kind: "percentage";
  /** 0-100 */
  percent: number;
}

export type Discount = FlatDiscount | PercentageDiscount;

export function applyDiscounts(subtotal: number, discounts: Discount[]): number {
  let remaining = subtotal;
  for (const discount of discounts) {
    if (discount.kind === "percentage") {
      remaining -= subtotal * (discount.percent / 100);
    } else {
      remaining -= discount.amount;
    }
  }
  return Math.max(0, remaining);
}
EOF

cat > src/promo-codes.ts <<'EOF'
import type { Discount } from "./discounts";

const PROMO_CODES: Record<string, Discount> = {
  LAUNCH10: { kind: "percentage", percent: 10 },
  WELCOME5: { kind: "flat", amount: 500 },
};

export function resolvePromoCode(code: string): Discount | undefined {
  return PROMO_CODES[code.trim().toUpperCase()];
}
EOF

cat > test/discounts.test.ts <<'EOF'
import { describe, expect, it } from "vitest";
import { applyDiscounts } from "../src/discounts";

describe("discounts", () => {
  it("applies a percentage discount", () => {
    expect(applyDiscounts(10000, [{ kind: "percentage", percent: 10 }])).toBe(
      9000,
    );
  });

  it("stacks percentage and flat discounts", () => {
    expect(
      applyDiscounts(10000, [
        { kind: "percentage", percent: 10 },
        { kind: "flat", amount: 500 },
      ]),
    ).toBe(8500);
  });

  it("never goes below zero", () => {
    expect(applyDiscounts(1000, [{ kind: "flat", amount: 2500 }])).toBe(0);
  });
});
EOF

# staged part: the tests; unstaged: discounts.ts; untracked: promo-codes.ts
git add test/discounts.test.ts

echo
echo "Demo repo ready at $TARGET"
echo "  - branch feature/checkout-discounts checked out, working tree dirty (hero state)"
echo "  - feature/messy-history for the interactive rebase shot"
echo "  - feature/tax-rounding conflicts with main on src/pricing.ts (merge main to trigger)"
echo "  - one stash entry"
