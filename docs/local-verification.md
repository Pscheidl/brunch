# Místní ověření 21. 10. 2026

Výsledek: všechny níže uvedené kontroly prošly. Neproběhl commit, push ani nasazení na vzdálený server.

## Prostředí

- Rust 1.98.1, Leptos 0.8.21, Trunk 0.21.14.
- Skutečný PostgreSQL 18.6 na `127.0.0.1:55434`.
- Místní Mailpit na SMTP portu 11026 a webu `http://127.0.0.1:18026`.
- Chromium přes Playwright 1.62.1, desktop 1440 px a mobil 390 px.
- Obchod a administrace na `http://127.0.0.1:3100`.

PostgreSQL pro toto ověření byl spuštěn nativně z dočasného adresáře `/tmp/brunch-runtime`, systémové služby se neměnily. Docker Compose je připraven pro opakovatelné vlastní spuštění, jeho běh zde ověřen nebyl. Účty a objednávky v místním náhledu jsou testovací, adresy používají doménu `example.test`. Zprávy zachycuje Mailpit a neodesílají se skutečným zákazníkům.

## Automatické kontroly

Závěrečný příkaz:

```bash
TEST_DATABASE_URL=postgres://brunch:brunch-local@127.0.0.1:55434/brunch CARGO_NET_OFFLINE=true ./scripts/test.sh
```

| Sada | Výsledek |
| --- | --- |
| Frontendové testy košíku a dopravy | 3 prošly |
| Backendové jednotkové testy | 6 prošlo |
| HTTP integrační scénáře na PostgreSQL | 8 prošlo |
| Skutečné SMTP, výpadek a opakování | 1 prošel |
| Bezpečnost relací, práv a poštovní fronty | 4 prošly |
| Nahrávání a poskytování obrázků | 2 prošly |
| Celkem | 24 prošlo, 0 selhalo, 0 přeskočeno v explicitně spuštěných sadách |
| Formátování, Clippy se zákazem varování | Prošlo |
| Optimalizované WebAssembly sestavení a kontrola souborů | Prošlo |
| Syntaxe administrace a kontrola změn | Prošlo |

Při sestavení frontendových závislostí Rust upozorňuje na budoucí nekompatibilitu balíčku `proc-macro-error2 2.0.1`. Aktuální připnutý toolchain sestavení dokončuje úspěšně.

## Skutečné prohlížečové ověření

`scripts/test-storefront-browser.cjs` prošel celý nákup bez účtu s hotovostí při doručení, doručení potvrzení do Mailpitu, registraci a přihlášení odkazem, nákup s účtem a kartou při doručení, vlastní historii, opakování objednávky, zachování košíku po obnovení stránky a odhlášení. Obchod ani objednávkový dialog nepřetékají při šířce 390 px. Žádná neošetřená chyba JavaScriptu.

`scripts/test-checkout-recovery.cjs` ověřil tři situace:

1. Katalogové API nebylo dostupné. Stránka zobrazila chybu a po výslovném opakování načetla skutečné produkty.
2. Server objednávku uložil, ale prohlížeč její odpověď ztratil. Opakování poslalo stejný obsah a klíč a vrátilo stejné ID. Přímá SQL kontrola našla právě jednu objednávku.
3. Prohlížeč zobrazil starší cenu. Server vrátil skutečný konflikt 409. Stránka obnovila nabídku a až další výslovné potvrzení vytvořilo objednávku za aktuální cenu. V databázi zůstal jeden záznam.

`scripts/test-admin-browser.cjs` ověřil skutečné přihlášení správce, nahrání a načtení fotografie, vytvoření produktu, změnu a historii ceny, archivaci, nastavení dopravy, hledání objednávky, adresu a pokyny, odkazy do mapy a telefonu, tisk rozvozového listu na jednu stranu A4, postupné změny až do doručení s potvrzením platby, zákazníky, e-mailovou frontu a mobilní administraci. Žádná neošetřená chyba JavaScriptu. Testovací produkt je po ověření archivovaný.

Snímky obrazovky a testovací PDF jsou v ignorovaných složkách `output/browser/` a `output/admin-browser/`. Nejsou součástí veřejného sestavení.

## Hranice ověření

Ověřeny jsou místní aplikace, PostgreSQL a SMTP. Produkční SMTP, HTTPS reverse proxy, skutečný platební terminál a provoz kurýrů nebyly připojeny. Zvolený termín je požadovaný čas doručení potvrzovaný obsluhou, nikoli rezervace rozvozové kapacity. Online platební brána zůstává podle zadání na pozdější verzi.
