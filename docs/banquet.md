# Banketní pečivo

V administraci v sekci **Zboží** lze u každého produktu zapnout **Banketní pečivo**. Takový produkt musí mít vyplněný vlastní minimální odběr od 1 do 99 kusů. U ostatního zboží je minimum nepovinné. Prázdné pole nebo nula znamená odběr od jednoho kusu.

V sekci **Obecné nastavení webu** je samostatný formulář **Banketní pečivo** s celkovým minimem za objednávku. Výchozí hodnota je **30 kusů**. Správce může nastavit celé číslo od 1 do 4950, což odpovídá nejvýše 50 různým položkám po 99 kusech v objednávce.

Do společného minima se započítávají pouze kusy produktů označených jako banketní pečivo. Například 10 housek se sýrem, 10 housek s lososem a 10 kusů s paštikou splní minimum 30, pokud jsou všechny tři produkty takto označené. Každá položka zároveň musí splnit své vlastní minimum. Běžné zboží se do součtu nepřičítá a objednávka bez banketního pečiva nemá společné kusové minimum.

Pravidla platí při doručení i osobním odběru. Košík ukazuje součet, potřebné minimum a chybějící počet. Server ověřuje aktuální hodnoty při přijetí objednávky v téže transakci jako dostupnost zboží. Neplatná objednávka neodečte sklad a nezařadí potvrzovací e-mail do fronty.

Ve společném týmovém košíku smí jednotliví kolegové přidat menší množství. Minima se kontrolují nad součtem celého týmu při uzavření, přenosu a následném objednání.

Migrace dosavadní produkty automaticky neoznačuje. Správce označí konkrétní zboží a nastaví jeho minimum. Změny nastavení ani označení produktů nepřepisují přijaté objednávky. Přesné opakování požadavku po ztrátě odpovědi nadále vrací již uloženou objednávku.

## Ověření

- `scripts/test.sh` zahrnuje databázovou a HTTP sadu `server/tests/banquet.rs`
- `scripts/test-banquet-admin-browser.cjs` ověřuje produktové a obecné nastavení
- `scripts/test-banquet-storefront-browser.cjs` ověřuje skutečný WASM košík s řízenými API odpověďmi
- `scripts/test-team-browser.cjs` ověřuje rozhraní společných objednávek

Pro databázové testy použijte izolovanou místní PostgreSQL podle README. Prohlížečové testy vyžadují Node.js a Playwright s Chromiem. Test obchodu vyžaduje aktuální sestavení v `dist`.

### Místní výsledek 21. 11. 2026

Prošlo 14 frontendových a 146 backendových testů, včetně čtyř nových databázových a HTTP skupin pro banketní pečivo. Prošly i nové prohlížečové sady administrace a obchodu, regrese společných košíků, osobního odběru, zákaznických funkcí a administračního nastavení. Ověřeny byly hranice 29/30 kusů, kombinace 10 + 10 + 10, vyloučení běžného zboží ze součtu, změny minima, zachování rozepsaných formulářů a bezpečné opakování po ztrátě odpovědi.

Prošlo formátování, Clippy frontendu i backendu se zákazem varování, optimalizované sestavení WASM a kontrola statických souborů. Mobilní košík i administrace byly ověřeny při šířkách 320 a 390 px. Testy používaly místní databázi a řízené API odpovědi, produkce se neměnila. Tato změna nebyla nasazena.
